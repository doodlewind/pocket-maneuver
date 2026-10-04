//! Storage: the pack's sections read one by one, the detailed cells read on
//! demand, the files beside the pack (the interface's bundle and what it
//! asked to have kept), and the PSPLINK mailbox.
//!
//! Two threads work here. The frame thread reads the pack's resident
//! sections straight into the memory that keeps them (`mem`): the base
//! machine has 24 MB. The thread the program started on serves the rest
//! (`serve`), at a lower priority than the frame's, so it runs while the frame
//! waits for the GE or the display and a read never delays a frame: a cell's
//! detailed meshes (`NEAR`) go into one of `SLOTS` buffers when the eye comes
//! near (a cell that has not arrived draws its simple mesh), the interface's
//! preferences go to their file, the mailbox is exchanged.
//!
//! The kernel keeps a working directory per thread, and only the thread the
//! program started on has one (the EBOOT's directory): a path without a
//! device fails anywhere else. So that thread opens what lies beside the
//! EBOOT before the frame thread starts (`locate`), and it is the one that
//! writes there.
//!
//! The frame thread owns every slot field except `state`, which the serving
//! thread moves from `LOADING` to `READY`. The serving thread cannot interrupt
//! the frame thread, so the frame thread's updates need no lock.

use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

use maneuver_handheld::world::World;
use maneuver_sim::math::V3;
use psp::sys::*;

use crate::mem;

pub struct Section {
    pub tag: u32,
    pub offset: u32,
    pub size: u32,
}

pub struct PackFile {
    pub fd: SceUid,
    pub sections: Vec<Section>,
    pub bytes: u32,
    /// Whether the computer's share is there (PSPLINK): the pack came from it, or it holds `maneuver/boot.txt`.
    pub host: bool,
}

/// The pack sits beside the EBOOT, and so do the files that go with it; otherwise they are on the
/// computer's share.
static mut LOCAL: bool = true;
/// What `locate` opened: the pack, the interface's script and pak, what the interface kept.
static mut FOUND: [SceUid; 4] = [SceUid(-1); 4];
const PACK: usize = 0;
pub const SCRIPT: usize = 1;
pub const PAK: usize = 2;
const KEPT: usize = 3;

/// Opens the pack and the files that go with it, beside the EBOOT (a packaged copy) or else on the
/// computer's share, and creates what `serve` waits on. Call on the thread the program started on,
/// before the frame thread exists. It allocates nothing.
pub unsafe fn locate() {
    const NAMES: [[&[u8]; 2]; 4] = [
        [b"world.pack\0", b"host0:/maneuver/world.pack\0"],
        [b"maneuver.js\0", b"host0:/maneuver/maneuver.js\0"],
        [b"maneuver.pak\0", b"host0:/maneuver/maneuver.pak\0"],
        [b"interface.json\0", b"host0:/maneuver/interface.json\0"],
    ];
    FOUND[PACK] = sceIoOpen(NAMES[PACK][0].as_ptr(), IoOpenFlags::RD_ONLY, 0);
    LOCAL = FOUND[PACK].0 >= 0;
    if !LOCAL {
        FOUND[PACK] = sceIoOpen(NAMES[PACK][1].as_ptr(), IoOpenFlags::RD_ONLY, 0);
    }
    for file in [SCRIPT, PAK, KEPT] {
        FOUND[file] = sceIoOpen(NAMES[file][!LOCAL as usize].as_ptr(), IoOpenFlags::RD_ONLY, 0);
    }
    WAKE = sceKernelCreateSema(b"maneuver_store\0".as_ptr(), 0, 0, 64, ptr::null_mut());
    LAST_CODE = WAKE.0.min(0);
}

/// One of the files `locate` opened, once: its handle and length.
unsafe fn found(file: usize) -> Option<(SceUid, usize)> {
    let fd = core::mem::replace(&mut FOUND[file], SceUid(-1));
    if fd.0 < 0 {
        return None;
    }
    let n = sceIoLseek32(fd, 0, IoWhence::End).max(0) as usize;
    sceIoLseek32(fd, 0, IoWhence::Set);
    Some((fd, n))
}

/// A file beside the pack (`SCRIPT`), whole, with `tail` appended, in memory the allocator takes back.
pub unsafe fn read_beside(file: usize, tail: &[u8]) -> Option<Vec<u8>> {
    let (fd, n) = found(file)?;
    let mut v = Vec::<u8>::new();
    let ok = n > 0 && v.try_reserve_exact(n + tail.len()).is_ok() && read_exact(fd, v.as_mut_ptr(), n);
    sceIoClose(fd);
    if !ok {
        return None;
    }
    v.set_len(n);
    v.extend_from_slice(tail);
    Some(v)
}

/// A file beside the pack (`PAK`), whole, in memory that keeps it for the rest of the run.
pub unsafe fn keep_beside(file: usize) -> Option<&'static [u8]> {
    let (fd, n) = found(file)?;
    let p = if n > 0 { mem::permanent(n) } else { ptr::null_mut() };
    let ok = !p.is_null() && read_exact(fd, p, n);
    sceIoClose(fd);
    ok.then(|| core::slice::from_raw_parts(p as *const u8, n))
}

/// Closes a file `locate` opened that nothing will read.
pub unsafe fn leave_beside(file: usize) {
    if let Some((fd, _)) = found(file) {
        sceIoClose(fd);
    }
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
    /// The pack `locate` opened: its section table.
    pub unsafe fn open() -> Result<PackFile, &'static str> {
        let (fd, bytes) = found(PACK).ok_or("no world.pack beside the program or on host0:/maneuver")?;
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
        // The mailbox is there when the computer's share has the game's directory, wherever the pack came from.
        let probe = sceIoOpen(b"host0:/maneuver/boot.txt\0".as_ptr(), IoOpenFlags::RD_ONLY, 0);
        if probe.0 >= 0 {
            sceIoClose(probe);
        }
        Ok(PackFile { fd, sections, bytes: bytes as u32, host: !LOCAL || probe.0 >= 0 })
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

    /// A whole section as `T` records (`T` is plain data), for the rest of the run, at its exact size.
    pub unsafe fn resident<T: Copy>(&self, tag: u32) -> Result<&'static [T], &'static str> {
        let s = self.section(tag)?;
        let n = s.size as usize / core::mem::size_of::<T>();
        let p = mem::permanent(n * core::mem::size_of::<T>());
        if p.is_null() {
            return Err("no memory for a section of the pack");
        }
        self.read_into(tag, 0, p, n * core::mem::size_of::<T>())?;
        Ok(core::slice::from_raw_parts(p as *const T, n))
    }

    /// The bytes of the sections that stay in memory, and of the buffers the detailed cells are read into.
    pub fn resident_bytes(&self) -> usize {
        let of = |tag| self.section(tag).map_or(0, |s| s.size as usize);
        [maneuver_pack::VTX0, maneuver_pack::IDX0, maneuver_pack::CLIP, maneuver_pack::MODL, maneuver_pack::FONT, maneuver_pack::SIMG].into_iter().map(of).sum::<usize>() + SLOTS * CELL_MAX
    }

    /// A small section as `T` records (`T` is plain data), from the allocator.
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
/// The compiler's limit on a cell's detailed meshes.
const CELL_MAX: usize = 48 * 1024;
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
static mut FD: SceUid = SceUid(-1);
static mut NEAR_BASE: u32 = 0;
static mut WAKE: SceUid = SceUid(-1);
static mut HOST: bool = false;
/// The kernel's answer when a start-up call fails, for the failure record.
pub static mut LAST_CODE: i32 = 0;
pub static LOADED: AtomicU32 = AtomicU32::new(0);
pub static LOADED_BYTES: AtomicU32 = AtomicU32::new(0);

// The mailbox: the frame thread hands a status text over and takes control text back.
const STATUS_MAX: usize = 4096;
static mut STATUS: [u8; STATUS_MAX] = [0; STATUS_MAX];
static mut STATUS_LEN: usize = 0;
static STATUS_FLAG: AtomicU32 = AtomicU32::new(0);
const CONTROL_MAX: usize = 512;
static mut CONTROL: [u8; CONTROL_MAX] = [0; CONTROL_MAX];
static mut CONTROL_LEN: usize = 0;
static CONTROL_FLAG: AtomicU32 = AtomicU32::new(0);
// What the interface asked to have kept, on its way to `interface.json` beside the pack.
pub const KEPT_MAX: usize = 1024;
static mut KEPT_TEXT: [u8; KEPT_MAX] = [0; KEPT_MAX];
static mut KEPT_LEN: usize = 0;
static KEPT_FLAG: AtomicU32 = AtomicU32::new(0);

/// Sets the detailed cells' buffers up over the pack's `NEAR` section and lets `serve` at the
/// mailbox. `largest` is the largest cell's bytes.
pub unsafe fn start(pack: &PackFile, largest: usize) -> Result<(), &'static str> {
    let per = (largest + 63) & !63;
    if per > 0 {
        let base = mem::permanent(per * SLOTS);
        if base.is_null() {
            return Err("no memory for the detailed cells");
        }
        for (i, s) in (*ptr::addr_of_mut!(SLOT)).iter_mut().enumerate() {
            s.data = base.add(i * per);
        }
        NEAR_BASE = pack.section(maneuver_pack::NEAR)?.offset;
    }
    FD = pack.fd;
    if WAKE.0 < 0 {
        return Err("the storage semaphore was not created");
    }
    HOST = pack.host;
    Ok(())
}

/// Serves storage for the rest of the run: the cells the frame thread asked for, what the
/// interface asked to have kept, and the mailbox. The thread the program started on runs it, below
/// the frame thread's priority.
pub unsafe fn serve() -> ! {
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
        if KEPT_FLAG.load(Ordering::Acquire) == 1 {
            let path: &[u8] = if LOCAL { b"interface.json\0" } else { b"host0:/maneuver/interface.json\0" };
            let fd = sceIoOpen(path.as_ptr(), IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::TRUNC, 0o644);
            if fd.0 >= 0 {
                sceIoWrite(fd, ptr::addr_of!(KEPT_TEXT) as *const c_void, KEPT_LEN);
                sceIoClose(fd);
            }
            KEPT_FLAG.store(0, Ordering::Release);
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

/// Hands what the interface asked to have kept to `serve`, which writes it beside the pack.
/// False while the previous one is still being written: the caller offers it again.
pub unsafe fn keep(text: &str) -> bool {
    if KEPT_FLAG.load(Ordering::Acquire) != 0 {
        return false;
    }
    let n = text.len().min(KEPT_MAX);
    ptr::copy_nonoverlapping(text.as_ptr(), ptr::addr_of_mut!(KEPT_TEXT) as *mut u8, n);
    KEPT_LEN = n;
    KEPT_FLAG.store(1, Ordering::Release);
    sceKernelSignalSema(WAKE, 1);
    true
}

/// What was kept in an earlier run, read once at the start.
pub unsafe fn kept(buf: &mut [u8]) -> Option<&str> {
    let (fd, _) = found(KEPT)?;
    let n = sceIoRead(fd, buf.as_mut_ptr() as *mut c_void, buf.len() as u32).max(0) as usize;
    sceIoClose(fd);
    core::str::from_utf8(&buf[..n]).ok()
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
