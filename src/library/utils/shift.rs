use crate::library::server::BUFFER_SIZE;
use std::arch::x86_64::{__m256i, _mm256_loadu_si256, _mm256_storeu_si256};

#[inline(always)]
pub unsafe fn shift_safe(buf: &mut [u8], len: usize) {
    // Behold! The sacred byte migration ritual.
    // Take the first `len` bytes, move them to the end,
    // zero the originals, and pray nothing explodes.
    for i in (0..len).rev() {
        // Move byte from front to back — manually, like peasants did in the dark ages
        *buf.get_unchecked_mut(buf.len() - len + i) = *buf.get_unchecked(i);
        // Clean up your mess
        *buf.get_unchecked_mut(i) = 0;
    }
}

#[inline(always)]
pub unsafe fn shift_ub(buf: &mut [u8], len: usize) -> [u8; BUFFER_SIZE] {
    // Vectorized arcane dance: lift the front, drop it at the end.
    // AVX2 priests bless this operation.
    let src: *const u8 = buf.as_ptr();
    // Allocate the holy destination altar (entirely zeroed, as is tradition)
    let mut dst_vec: [u8; BUFFER_SIZE] = [0u8; BUFFER_SIZE];
    let dst: *mut u8 = dst_vec.as_mut_ptr().add(BUFFER_SIZE - len);
    // Bring in the AVX2 death squad
    let mut i = 0;
    while i + 32 <= len {
        let chunk: __m256i = _mm256_loadu_si256(src.add(i) as *const __m256i);
        _mm256_storeu_si256(dst.add(i) as *mut __m256i, chunk);
        i += 32;
    }
    // Handle the pitiful stragglers without SIMD superpowers
    while i < len {
        *dst.add(i) = *src.add(i);
        i += 1;
    }
    // Return the relic to the calling realm
    dst_vec
}
