//! The global allocator.
//!
//! rust-psp's own allocator takes one kernel block per allocation, and the
//! kernel allows about 4096 objects. Here an allocation of 64 KiB or more (or
//! one aligned beyond 16 bytes) is a kernel block of its exact size: the
//! collision grid and the vertex buffers are a dozen such blocks and must not
//! be rounded up. Everything smaller comes from one 2 MiB block, in
//! power-of-two classes with a free list each.
//!
//! Only the main thread allocates.

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::ptr;

use psp::sys::{self, SceSysMemBlockTypes, SceSysMemPartitionId, SceUid};

const ARENA_BYTES: usize = 2 * 1024 * 1024;
const LARGE: usize = 64 * 1024;
const MIN_SHIFT: usize = 4;
const CLASSES: usize = 17;
const BLOCKS: usize = 128;

static mut FREE: [*mut u8; CLASSES] = [ptr::null_mut(); CLASSES];
static mut BASE: usize = 0;
static mut BUMP: usize = 0;
static mut END: usize = 0;
static mut LARGE_BLOCKS: [(usize, i32); BLOCKS] = [(0, 0); BLOCKS];
pub static mut LARGE_BYTES: usize = 0;

unsafe fn kernel_block(size: usize) -> (usize, i32) {
    let id = sys::sceKernelAllocPartitionMemory(SceSysMemPartitionId::SceKernelPrimaryUserPartition, b"maneuver\0".as_ptr(), SceSysMemBlockTypes::Low, size as u32, ptr::null_mut::<c_void>());
    if id.0 < 0 {
        return (0, -1);
    }
    (sys::sceKernelGetBlockHeadAddr(id) as usize, id.0)
}

fn class_of(size: usize) -> usize {
    let mut c = MIN_SHIFT;
    while (1usize << c) < size {
        c += 1;
    }
    c
}

/// Bytes of the small-allocation block in use.
pub fn arena_used() -> usize {
    unsafe { BUMP - BASE }
}

struct Alloc;

unsafe impl GlobalAlloc for Alloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(1);
        if size >= LARGE || layout.align() > 16 {
            // Kernel blocks start on 256-byte boundaries.
            if layout.align() > 256 {
                return ptr::null_mut();
            }
            let slot = match (*ptr::addr_of!(LARGE_BLOCKS)).iter().position(|b| b.0 == 0) {
                Some(s) => s,
                None => return ptr::null_mut(),
            };
            let (addr, id) = kernel_block(size);
            if addr == 0 {
                return ptr::null_mut();
            }
            LARGE_BLOCKS[slot] = (addr, id);
            LARGE_BYTES += size;
            return addr as *mut u8;
        }
        if BASE == 0 {
            let (addr, _) = kernel_block(ARENA_BYTES);
            if addr == 0 {
                return ptr::null_mut();
            }
            BASE = (addr + 15) & !15;
            BUMP = BASE;
            END = addr + ARENA_BYTES;
        }
        let c = class_of(size);
        let head = FREE[c - MIN_SHIFT];
        if !head.is_null() {
            FREE[c - MIN_SHIFT] = *(head as *mut *mut u8);
            return head;
        }
        if BUMP + (1 << c) > END {
            return ptr::null_mut();
        }
        let p = BUMP as *mut u8;
        BUMP += 1 << c;
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        let addr = p as usize;
        if addr >= BASE && addr < END && BASE != 0 {
            let c = class_of(layout.size().max(1));
            *(p as *mut *mut u8) = FREE[c - MIN_SHIFT];
            FREE[c - MIN_SHIFT] = p;
            return;
        }
        if let Some(b) = (*ptr::addr_of_mut!(LARGE_BLOCKS)).iter_mut().find(|b| b.0 == addr) {
            sys::sceKernelFreePartitionMemory(SceUid(b.1));
            *b = (0, 0);
            LARGE_BYTES -= layout.size();
        }
    }
}

#[global_allocator]
static GLOBAL: Alloc = Alloc;

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    psp::dprintln!("out of memory: {} bytes", layout.size());
    loop {
        unsafe { sys::sceKernelDelayThread(1_000_000) };
    }
}
