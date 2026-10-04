//! Memory at its exact size.
//!
//! PocketJS's arena is the program's allocator (`pocketjs-psp` installs it):
//! one kernel block, handed out in power-of-two classes so that QuickJS's
//! churn recycles in constant time. A 4.8 MB vertex buffer from it would take
//! 8 MB. The world's buffers live for the whole run and are never given back,
//! so they come from here: first from the 2 MB the arena leaves the kernel
//! (as kernel blocks, down to `KEEP`), then from the arena's uncarved tail.
//!
//! Only the frame thread allocates.

use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr;

use pocketjs_psp::arena;
use psp::sys::{self, SceSysMemBlockTypes, SceSysMemPartitionId, SceUid};

/// What stays with the kernel: the stacks of the threads started after loading (the reader's, the
/// sound's) and its own objects.
const KEEP: usize = 320 * 1024;
/// Below this a request is not worth a kernel object.
const SMALL: usize = 64 * 1024;

/// Bytes held in kernel blocks.
pub static mut KERNEL_BYTES: usize = 0;

/// A kernel block of `bytes` while the kernel keeps `KEEP` after it: its address and id.
unsafe fn kernel_block(bytes: usize) -> Option<(*mut u8, SceUid)> {
    // The arena takes its block on first use; until then the kernel's free memory is not what is left over.
    let _ = arena::stats();
    if (sys::sceKernelMaxFreeMemSize() as usize) < bytes + KEEP {
        return None;
    }
    let id = sys::sceKernelAllocPartitionMemory(SceSysMemPartitionId::SceKernelPrimaryUserPartition, b"maneuver\0".as_ptr(), SceSysMemBlockTypes::Low, bytes as u32, ptr::null_mut::<c_void>());
    if id.0 < 0 {
        return None;
    }
    let p = sys::sceKernelGetBlockHeadAddr(id) as *mut u8;
    (!p.is_null()).then_some((p, id))
}

/// `bytes` of uninitialized memory for the rest of the run, 64-byte aligned. Null when there is none.
pub unsafe fn permanent(bytes: usize) -> *mut u8 {
    let bytes = bytes.max(16);
    if bytes >= SMALL {
        // Kernel blocks start on 256-byte boundaries.
        if let Some((p, _)) = kernel_block(bytes) {
            KERNEL_BYTES += bytes;
            return p;
        }
    }
    arena::alloc_permanent(bytes, 64)
}

/// A vector of capacity `n` over permanent memory, empty. It must live for the rest of the run and
/// never grow: the allocator did not hand this memory out and cannot take it back.
pub unsafe fn vec<T: Copy>(n: usize) -> Option<Vec<T>> {
    let p = permanent(n.max(1) * core::mem::size_of::<T>());
    (!p.is_null()).then(|| Vec::from_raw_parts(p.cast(), 0, n.max(1)))
}

/// Runs `f` over `bytes` of memory that is given back afterwards: a kernel block when the kernel
/// has one, so that a large buffer needed once does not stay behind in an arena class.
pub unsafe fn scratch<R>(bytes: usize, f: impl FnOnce(&mut [u8]) -> R) -> Option<R> {
    if let Some((p, id)) = kernel_block(bytes) {
        let r = f(core::slice::from_raw_parts_mut(p, bytes));
        sys::sceKernelFreePartitionMemory(id);
        return Some(r);
    }
    let mut v = Vec::<u8>::new();
    v.try_reserve_exact(bytes).ok()?;
    v.set_len(bytes);
    Some(f(&mut v))
}

/// Bytes the kernel can still give as one block, beyond what it keeps.
pub unsafe fn kernel_room() -> usize {
    let _ = arena::stats();
    (sys::sceKernelMaxFreeMemSize() as usize).saturating_sub(KEEP)
}
