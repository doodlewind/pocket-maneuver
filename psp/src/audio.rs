//! Sound: the frame thread renders the simulation's synthesizer into a ring;
//! a thread with a higher priority moves blocks from the ring to the output.
//! It only copies, so it holds the frame up for microseconds.

use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

use psp::sys::*;
use psp::Align16;

/// Frames per second of the synthesizer and the output channel.
pub const RATE: f32 = 22050.0;
const BLOCK: usize = 512;
/// Frames in the ring (a power of two).
const RING: usize = 4096;

static mut BUF: [i16; RING * 2] = [0; RING * 2];
static WRITE: AtomicU32 = AtomicU32::new(0);
static READ: AtomicU32 = AtomicU32::new(0);
static RUNNING: AtomicU32 = AtomicU32::new(0);

pub unsafe fn start() -> bool {
    let id = sceKernelCreateThread(b"maneuver_audio\0".as_ptr(), output, 24, 16 * 1024, ThreadAttributes::USER, ptr::null_mut());
    if id.0 < 0 {
        return false;
    }
    sceKernelStartThread(id, 0, ptr::null_mut());
    true
}

pub fn running() -> bool {
    RUNNING.load(Ordering::Relaxed) == 1
}

unsafe extern "C" fn output(_: usize, _: *mut c_void) -> i32 {
    if sceAudioSRCChReserve(BLOCK as i32, AudioOutputFrequency::Khz22_05, 2) < 0 {
        return 0;
    }
    RUNNING.store(1, Ordering::Relaxed);
    let mut block = Align16([0i16; BLOCK * 2]);
    loop {
        let (r, w) = (READ.load(Ordering::Acquire), WRITE.load(Ordering::Acquire));
        if w.wrapping_sub(r) as usize >= BLOCK {
            for i in 0..BLOCK {
                let at = (r as usize + i) % RING * 2;
                block.0[i * 2] = BUF[at];
                block.0[i * 2 + 1] = BUF[at + 1];
            }
            READ.store(r.wrapping_add(BLOCK as u32), Ordering::Release);
        } else {
            // The frame thread fell behind: silence, not a repeat.
            block.0 = [0; BLOCK * 2];
        }
        sceAudioSRCOutputBlocking(0x8000, block.0.as_mut_ptr() as *mut c_void);
    }
}

/// Frames the frame thread should render now to keep about three blocks queued.
pub fn wanted() -> usize {
    let queued = WRITE.load(Ordering::Acquire).wrapping_sub(READ.load(Ordering::Acquire)) as usize;
    (BLOCK * 3).saturating_sub(queued).min(1024)
}

/// Queues interleaved stereo frames.
pub unsafe fn push(pcm: &[i16]) {
    let w = WRITE.load(Ordering::Acquire);
    let frames = pcm.len() / 2;
    for i in 0..frames {
        let at = (w as usize + i) % RING * 2;
        BUF[at] = pcm[i * 2];
        BUF[at + 1] = pcm[i * 2 + 1];
    }
    WRITE.store(w.wrapping_add(frames as u32), Ordering::Release);
}
