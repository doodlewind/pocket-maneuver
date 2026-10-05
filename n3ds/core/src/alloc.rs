//! The global allocator over the C library's heap (newlib on the 3DS, the
//! system's on the iPod touch, whose core is this same source), and the
//! handlers a `no_std` static library provides itself.

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;

/// Alignment `malloc` guarantees on both (newlib on devkitARM; iOS gives 16).
const C_ALIGN: usize = 8;

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    #[cfg(not(target_vendor = "apple"))]
    fn memalign(alignment: usize, size: usize) -> *mut c_void;
    #[cfg(target_vendor = "apple")]
    fn posix_memalign(out: *mut *mut c_void, alignment: usize, size: usize) -> i32;
    fn free(ptr: *mut c_void);
    fn abort() -> !;
}

/// A block aligned beyond `C_ALIGN`.
#[cfg(not(target_vendor = "apple"))]
unsafe fn aligned(alignment: usize, size: usize) -> *mut c_void {
    memalign(alignment, size)
}

#[cfg(target_vendor = "apple")]
unsafe fn aligned(alignment: usize, size: usize) -> *mut c_void {
    let mut block = core::ptr::null_mut();
    if posix_memalign(&mut block, alignment, size) != 0 {
        return core::ptr::null_mut();
    }
    block
}

struct CAllocator;

unsafe impl GlobalAlloc for CAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.align() <= C_ALIGN {
            malloc(layout.size().max(1)).cast()
        } else {
            aligned(layout.align(), layout.size().max(1)).cast()
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
