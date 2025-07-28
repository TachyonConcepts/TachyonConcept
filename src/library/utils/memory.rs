use std::{
    arch::x86_64::{__m256i, _mm256_setzero_si256, _mm256_storeu_si256},
    ptr,
};

// Flattens a slice of iovecs into a single `&mut [u8]`.
// Designed to be stupidly fast, entirely unsafe, and 100% fearless.
// This function assumes that `out` is big enough. If it’s not — you’re on your own.
// You will experience UB, segfaults, and possibly a visit from kernel gods.
#[inline(always)]
pub unsafe fn fast_flatten_iovec(iovecs: &[libc::iovec], out: &mut [u8]) -> usize {
    let mut offset = 0;
    for iov in iovecs {
        let len = iov.iov_len;
        // Copy the contents of each iovec directly into the output buffer
        // No checks, no ceremony, just raw bytes moving like they were born to.
        ptr::copy_nonoverlapping(iov.iov_base as *const u8, out.as_mut_ptr().add(offset), len);
        offset += len; // Move the offset forward like a conveyor belt of danger
    }
    offset // Return total bytes written (so you can pretend it’s safe)
}

// Same as above, but plays slightly nicer — uses a Vec and expands it as needed.
// Still unsafe. Still not for mortals. But hey, at least the Vec will reserve enough space.
// This is what you call when you want a heap-allocated one-liner that says: “give me ALL the data”.
#[inline(always)]
pub unsafe fn flatten_iovec(iovecs: &[libc::iovec], out: &mut Vec<u8>) {
    let total_len: usize = iovecs.iter().map(|iov| iov.iov_len).sum();
    let orig_len = out.len();
    out.reserve(total_len); // Politely ask the heap for more RAM
    unsafe {
        // okay, no more politeness from here on
        out.set_len(orig_len + total_len); // Expand the Vec without initializing memory.
        let mut dst = out.as_mut_ptr().add(orig_len);
        for iov in iovecs {
            let src = iov.iov_base as *const u8;
            ptr::copy_nonoverlapping(src, dst, iov.iov_len); // Memory flies. Sanity cries.
            dst = dst.add(iov.iov_len);
        }
    }
}

// Purpose: Obliterate a buffer using 256-bit AVX2 stores like a memory-purging demon.
// It's like memset, but instead of C heritage, it brings raw unaligned SIMD firepower.
// Faster than `ptr::write_bytes`, cooler than `memset`, and 100% less safe.
// Requirements:
// - AVX2-capable CPU
// - You know what you’re doing (you probably don’t)
// - Alignment? Who cares? We're using unaligned stores and hoping for the best.
#[target_feature(enable = "avx2")]
pub unsafe fn avx2_zero(buf: *mut u8, len: usize) {
    let mut ptr = buf;
    let end = buf.add(len);
    // Set up a 256-bit register full of zeroes. That’s 32 zero bytes at a time.
    let zero = _mm256_setzero_si256();
    // Main loop: dump zeroes into memory 32 bytes at a time.
    while ptr.add(32) <= end {
        _mm256_storeu_si256(ptr as *mut __m256i, zero); // Unaligned store — YOLO
        ptr = ptr.add(32); // Move forward like a buffer-burning freight train
    }
    // If there's any sad leftover tail (< 32 bytes), clean it the boring way.
    if ptr < end {
        ptr::write_bytes(ptr, 0, end.offset_from(ptr) as usize);
    }
}
