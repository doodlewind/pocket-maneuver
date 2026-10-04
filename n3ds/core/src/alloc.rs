//! The global allocator over newlib's heap, and the handlers a `no_std`
//! static library provides itself.

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;

/// Alignment newlib's `malloc` guarantees on devkitARM.
const C_ALIGN: usize = 8;

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn memalign(alignment: usize, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn abort() -> !;
}

struct CAllocator;

unsafe impl GlobalAlloc for CAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.align() <= C_ALIGN {
            malloc(layout.size().max(1)).cast()
        } else {
            memalign(layout.align(), layout.size().max(1)).cast()
        }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        free(ptr.cast());
    }
}

#[global_allocator]
static GLOBAL: CAllocator = CAllocator;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { abort() }
}
