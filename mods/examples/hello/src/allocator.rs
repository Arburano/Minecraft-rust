// SAFETY: ADR-0012 confines the canonical-ABI pointer contract to this guest module.
#![allow(unsafe_code)]

extern crate alloc;

use alloc::alloc::{Layout, alloc, dealloc, handle_alloc_error, realloc};

// SAFETY: This is the sole cabi_realloc export; the host supplies canonical-ABI allocations.
#[unsafe(no_mangle)]
unsafe extern "C" fn cabi_realloc(
    old_ptr: *mut u8,
    old_len: usize,
    align: usize,
    new_len: usize,
) -> *mut u8 {
    let layout = Layout::from_size_align(new_len, align)
        .unwrap_or_else(|_| core::arch::wasm32::unreachable());
    let old_layout = Layout::from_size_align(old_len, align)
        .unwrap_or_else(|_| core::arch::wasm32::unreachable());
    // SAFETY: Nonempty old_ptr is a live allocation of old_layout from this allocator (canonical ABI).
    let ptr = unsafe {
        if new_len == 0 {
            if old_len != 0 {
                dealloc(old_ptr, old_layout);
            }
            return align as *mut u8;
        }
        if old_len == 0 {
            alloc(layout)
        } else {
            realloc(old_ptr, old_layout, new_len)
        }
    };
    if ptr.is_null() {
        handle_alloc_error(layout);
    }
    ptr
}
