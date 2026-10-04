//! Storage: the pack's sections read one by one, the detailed cells read on
//! demand by a second thread, and the PSPLINK mailbox the same thread serves.
//!
//! The machine has 24 MB. The pack's resident sections are read straight into
//! the memory that keeps them. A cell's detailed meshes (`NEAR`) are read into
//! one of `SLOTS` buffers when the eye comes near, by a thread with a lower
//! priority than the frame's: it runs while the frame waits for the GE or the
//! display, so a read never delays a frame. A cell that has not arrived draws
//! its simple mesh.
//!
//! The frame thread owns every slot field except `state`, which the reader
//! moves from `LOADING` to `READY`. The reader cannot interrupt the frame
//! thread, so the frame thread's updates need no lock.

use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

use maneuver_handheld::world::World;
use maneuver_sim::math::V3;
use psp::sys::*;

pub struct Section {
    pub tag: u32,
    pub offset: u32,
    pub size: u32,
}

pub struct PackFile {
    pub fd: SceUid,
    pub sections: Vec<Section>,
    pub bytes: u32,
    /// Whether the pack came from the computer over PSPLINK.
    pub host: bool,
}

/// Reads exactly `len` bytes; storage returns short reads on large requests.
pub unsafe fn read_exact(fd: SceUid, dst: *mut u8, len: usize) -> bool {
    let mut at = 0;
    while at < len {
        let n = sceIoRead(fd, dst.add(at) as *mut c_void, (len - at).min(256 * 1024) as u32);
        if n <= 0 {
            return false;
        }
        at += n as usize;
    }
    true
}

impl PackFile {
    pub unsafe fn open() -> Result<PackFile, &'static str> {
        // Beside the EBOOT first (a packaged copy), then the computer's share.
        for (path, host) in [(&b"world.pack\0"[..], false), (&b"host0:/maneuver/world.pack\0"[..], true)] {
            let fd = sceIoOpen(path.as_ptr(), IoOpenFlags::RD_ONLY, 0);
            if fd.0 < 0 {
                continue;
            }
            let bytes = sceIoLseek32(fd, 0, IoWhence::End).max(0) as u32;
            sceIoLseek32(fd, 0, IoWhence::Set);
            let mut head = [0u32; 4];
            if !read_exact(fd, head.as_mut_ptr().cast(), 16) || head[0] != maneuver_pack::MAGIC {
                return Err("world.pack is not a pack");
            }
            if head[1] != maneuver_pack::VERSION {
                return Err("world.pack has another version");
            }
            let n = head[2] as usize;
            if n > 64 {
                return Err("world.pack section table");
            }
            let mut table = alloc::vec![0u32; n * 4];
            if !read_exact(fd, table.as_mut_ptr().cast(), n * 16) {
                return Err("world.pack section table");
            }
            let sections = (0..n).map(|i| Section { tag: table[i * 4], offset: table[i * 4 + 1], size: table[i * 4 + 2] }).collect();
            return Ok(PackFile { fd, sections, bytes, host });
        }
        Err("no world.pack beside the program or on host0:/maneuver")
    }

    pub fn section(&self, tag: u32) -> Result<&Section, &'static str> {
        self.sections.iter().find(|s| s.tag == tag).ok_or("the pack lacks a section")
    }

    /// Reads `len` bytes at `offset` of a section into `dst`.
    pub unsafe fn read_into(&self, tag: u32, offset: usize, dst: *mut u8, len: usize) -> Result<(), &'static str> {
        let s = self.section(tag)?;
        if offset + len > s.size as usize {
            return Err("read past a section's end");
        }
        sceIoLseek32(self.fd, (s.offset as usize + offset) as i32, IoWhence::Set);
        if read_exact(self.fd, dst, len) {
            Ok(())
        } else {
            Err("reading the pack failed")
        }
    }

    /// A whole section as `T` records (`T` is plain data).
    pub unsafe fn records<T: Copy>(&self, tag: u32) -> Result<Vec<T>, &'static str> {
        let s = self.section(tag)?;
        let n = s.size as usize / core::mem::size_of::<T>();
        let mut v = Vec::<T>::with_capacity(n.max(1));
        self.read_into(tag, 0, v.as_mut_ptr().cast(), n * core::mem::size_of::<T>())?;
        v.set_len(n);
        Ok(v)
    }
}

// ---------------------------------------------------------------- on-demand cells

pub const SLOTS: usize = 16;
const FREE: u32 = 0;
const LOADING: u32 = 1;
const READY: u32 = 2;
const NONE: u32 = u32::MAX;

struct Slot {
    cell: u32,
    state: AtomicU32,
    /// The last frame that drew from this slot.
    used: u32,
    offset: u32,
    len: u32,
    data: *mut u8,
}

const EMPTY: Slot = Slot { cell: NONE, state: AtomicU32::new(FREE), used: 0, offset: 0, len: 0, data: ptr::null_mut() };
static mut SLOT: [Slot; SLOTS] = [EMPTY; SLOTS];
static mut CACHE: Vec<u8> = Vec::new();
static mut FD: SceUid = SceUid(-1);
static mut NEAR_BASE: u32 = 0;
static mut WAKE: SceUid = SceUid(-1);
static mut HOST: bool = false;
pub static LOADED: AtomicU32 = AtomicU32::new(0);
pub static LOADED_BYTES: AtomicU32 = AtomicU32::new(0);

// The mailbox: the frame thread hands a status text over and takes control text back.
const STATUS_MAX: usize = 2048;
static mut STATUS: [u8; STATUS_MAX] = [0; STATUS_MAX];
static mut STATUS_LEN: usize = 0;
static STATUS_FLAG: AtomicU32 = AtomicU32::new(0);
const CONTROL_MAX: usize = 512;
static mut CONTROL: [u8; CONTROL_MAX] = [0; CONTROL_MAX];
static mut CONTROL_LEN: usize = 0;
static CONTROL_FLAG: AtomicU32 = AtomicU32::new(0);

/// Starts the reader over the pack's `NEAR` section. `largest` is the largest cell's bytes.
pub unsafe fn start(pack: &PackFile, largest: usize) -> Result<(), &'static str> {
    let per = (largest + 63) & !63;
    if per > 0 {
        CACHE = alloc::vec![0u8; per * SLOTS];
        let base = (*ptr::addr_of_mut!(CACHE)).as_mut_ptr();
        for (i, s) in (*ptr::addr_of_mut!(SLOT)).iter_mut().enumerate() {
            s.data = base.add(i * per);
        }
        NEAR_BASE = pack.section(maneuver_pack::NEAR)?.offset;
    }
    FD = pack.fd;
    HOST = pack.host;
    WAKE = sceKernelCreateSema(b"maneuver_read\0".as_ptr(), 0, 0, 64, ptr::null_mut());
    let id = sceKernelCreateThread(b"maneuver_read\0".as_ptr(), reader, 40, 32 * 1024, ThreadAttributes::USER, ptr::null_mut());
    if id.0 < 0 || WAKE.0 < 0 {
        return Err("the reader thread did not start");
    }
    sceKernelStartThread(id, 0, ptr::null_mut());
    Ok(())
}

unsafe extern "C" fn reader(_: usize, _: *mut c_void) -> i32 {
    let mut last_control = [0u8; CONTROL_MAX];
    let mut last_len = usize::MAX;
    let mut beat = 0u32;
    loop {
        let mut timeout = 200_000u32;
        sceKernelWaitSema(WAKE, 1, &mut timeout);
        for s in (*ptr::addr_of_mut!(SLOT)).iter_mut() {
            if s.state.load(Ordering::Relaxed) != LOADING {
                continue;
            }
            sceIoLseek32(FD, (NEAR_BASE + s.offset) as i32, IoWhence::Set);
            if read_exact(FD, s.data, s.len as usize) {
                // The GE reads memory, not the data cache.
                sceKernelDcacheWritebackRange(s.data as *const c_void, s.len);
                LOADED.fetch_add(1, Ordering::Relaxed);
                LOADED_BYTES.fetch_add(s.len, Ordering::Relaxed);
                s.state.store(READY, Ordering::Release);
            } else {
                // Leave it free: the frame asks again.
                s.cell = NONE;
                s.state.store(FREE, Ordering::Release);
            }
        }
        if !HOST {
            continue;
        }
        if STATUS_FLAG.load(Ordering::Acquire) == 1 {
            let fd = sceIoOpen(b"host0:/maneuver/status.json\0".as_ptr(), IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::TRUNC, 0o644);
            if fd.0 >= 0 {
                sceIoWrite(fd, ptr::addr_of!(STATUS) as *const c_void, STATUS_LEN);
                sceIoClose(fd);
            }
            STATUS_FLAG.store(0, Ordering::Release);
        }
        beat += 1;
        if beat % 2 == 0 && CONTROL_FLAG.load(Ordering::Acquire) == 0 {
            let fd = sceIoOpen(b"host0:/maneuver/control.txt\0".as_ptr(), IoOpenFlags::RD_ONLY, 0);
            if fd.0 >= 0 {
                let mut buf = [0u8; CONTROL_MAX];
                let n = sceIoRead(fd, buf.as_mut_ptr() as *mut c_void, CONTROL_MAX as u32).max(0) as usize;
                sceIoClose(fd);
                // What is there at launch is left over from an earlier run: only changes after it count.
                if last_len == usize::MAX {
                    last_control = buf;
                    last_len = n;
                } else if n != last_len || buf[..n] != last_control[..n] {
                    last_control = buf;
                    last_len = n;
                    CONTROL = buf;
                    CONTROL_LEN = n;
                    CONTROL_FLAG.store(1, Ordering::Release);
                }
            }
        }
    }
}

/// The buffer holding `cell`'s detailed meshes, if it has arrived; marks it drawn this frame.
pub unsafe fn data(cell: u32, frame: u32) -> Option<(*const u8, u32)> {
    for s in (*ptr::addr_of_mut!(SLOT)).iter_mut() {
        if s.cell == cell && s.state.load(Ordering::Acquire) == READY {
            s.used = frame;
            return Some((s.data, s.offset));
        }
    }
    None
}

pub unsafe fn ready(cell: u32) -> bool {
    (*ptr::addr_of!(SLOT)).iter().any(|s| s.cell == cell && s.state.load(Ordering::Acquire) == READY)
}

/// Asks for the cells near `eye`, nearest first, replacing ones that are out of range.
/// Call while the GE is idle: a replaced buffer must not be in a list it is still drawing.
pub unsafe fn update(world: &World, eye: V3, radius: f32, frame: u32, wanted: &mut Vec<(f32, u32)>) {
    wanted.clear();
    world.wanted(eye, radius, wanted);
    wanted.sort_unstable_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
    wanted.truncate(SLOTS);
    let slots = &mut *ptr::addr_of_mut!(SLOT);
    let mut asked = false;
    for &(_, cell) in wanted.iter() {
        if slots.iter().any(|s| s.cell == cell && s.state.load(Ordering::Acquire) != FREE) {
            continue;
        }
        // A free buffer, or the ready one whose cell is no longer wanted and that no recent frame drew.
        let pick = slots.iter().position(|s| s.state.load(Ordering::Acquire) == FREE).or_else(|| {
            slots.iter().position(|s| s.state.load(Ordering::Acquire) == READY && frame.wrapping_sub(s.used) >= 3 && !wanted.iter().any(|w| w.1 == s.cell))
        });
        let Some(i) = pick else { break };
        let blob = world.cells[cell as usize].blob;
        let s = &mut slots[i];
        s.cell = cell;
        s.offset = blob.0;
        s.len = blob.1;
        s.used = frame;
        s.state.store(LOADING, Ordering::Release);
        asked = true;
    }
    if asked {
        sceKernelSignalSema(WAKE, 1);
    }
}

/// Hands a status record to the mailbox; dropped when the previous one is still being written.
pub unsafe fn publish(text: &str) {
    if !HOST || STATUS_FLAG.load(Ordering::Acquire) != 0 {
        return;
    }
    let n = text.len().min(STATUS_MAX);
    ptr::copy_nonoverlapping(text.as_ptr(), ptr::addr_of_mut!(STATUS) as *mut u8, n);
    STATUS_LEN = n;
    STATUS_FLAG.store(1, Ordering::Release);
}

/// New control text from the computer, once.
pub unsafe fn control(f: impl FnOnce(&str)) {
    if CONTROL_FLAG.load(Ordering::Acquire) != 1 {
        return;
    }
    let bytes = core::slice::from_raw_parts(ptr::addr_of!(CONTROL) as *const u8, CONTROL_LEN);
    if let Ok(text) = core::str::from_utf8(bytes) {
        f(text);
    }
    CONTROL_FLAG.store(0, Ordering::Release);
}

/// A small text file from the computer, read once (`host0:/maneuver/boot.txt`: control words applied at start).
pub unsafe fn boot_text(buf: &mut [u8]) -> Option<&str> {
    let fd = sceIoOpen(b"host0:/maneuver/boot.txt\0".as_ptr(), IoOpenFlags::RD_ONLY, 0);
    if fd.0 < 0 {
        return None;
    }
    let n = sceIoRead(fd, buf.as_mut_ptr() as *mut c_void, buf.len() as u32).max(0) as usize;
    sceIoClose(fd);
    core::str::from_utf8(&buf[..n]).ok()
}

/// Writes a frame the frame thread copied out of video memory to `host0:/maneuver/shot.raw`.
pub unsafe fn write_shot(pixels: &[u8]) {
    let fd = sceIoOpen(b"host0:/maneuver/shot.raw\0".as_ptr(), IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::TRUNC, 0o644);
    if fd.0 >= 0 {
        let mut at = 0;
        while at < pixels.len() {
            let n = sceIoWrite(fd, pixels.as_ptr().add(at) as *const c_void, (pixels.len() - at).min(64 * 1024));
            if n <= 0 {
                break;
            }
            at += n as usize;
        }
        sceIoClose(fd);
    }
}

/// Writes one message for the computer while loading (the frame loop is not running yet).
pub unsafe fn note(text: &str) {
    let fd = sceIoOpen(b"host0:/maneuver/status.json\0".as_ptr(), IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::TRUNC, 0o644);
    if fd.0 >= 0 {
        sceIoWrite(fd, text.as_ptr() as *const c_void, text.len());
        sceIoClose(fd);
    }
}
