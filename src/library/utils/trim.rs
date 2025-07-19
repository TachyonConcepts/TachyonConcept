use std::arch::x86_64::{
    __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_set1_epi8,
    _mm256_setzero_si256,
};

#[inline(always)]
pub unsafe fn l_trim256(data: &[u8]) -> &[u8] {
    // AVX2-enhanced left trim.
    // Deletes leading zeros with the finesse of a laser-guided byte scalpel.
    let len = data.len();
    let ptr = data.as_ptr();
    let mut i = 0;
    let zero = _mm256_set1_epi8(0);
    while i + 32 <= len {
        let chunk = _mm256_loadu_si256(ptr.add(i) as *const __m256i);
        let cmp = _mm256_cmpeq_epi8(chunk, zero);
        let mask = _mm256_movemask_epi8(cmp);
        // Found some non-zero? Great! Slice off the dead weight.
        if mask != -1 {
            let tz = mask.trailing_ones() as usize;
            return &data[i + tz..];
        }
        // Still all zeros. Sigh. Move on.
        i += 32;
    }
    // Handle pathetic leftover bytes one by one. Yes, we're above this, but we do it anyway.
    while i < len {
        if *ptr.add(i) != 0 {
            return &data[i..];
        }
        i += 1;
    }
    // Nothing but zeros. Send back an empty slice and a therapy bill.
    &[]
}

#[inline(always)]
pub unsafe fn r_trim256(buf: &[u8]) -> &[u8] {
    // AVX2-empowered right trim.
    // Because right-aligned zeroes deserve to be eliminated too.
    let len = buf.len();
    if len == 0 {
        return &[];
    }
    let ptr = buf.as_ptr();
    let mut i = len;
    while i >= 32 {
        let offset = i - 32;
        let chunk = _mm256_loadu_si256(ptr.add(offset) as *const __m256i);
        let cmp = _mm256_cmpeq_epi8(chunk, _mm256_setzero_si256());
        let mask = _mm256_movemask_epi8(cmp);
        // Found bytes that aren't zero? Sweet, let's stop pretending the rest matters.
        if mask != -1 {
            for j in (0..32).rev() {
                if *ptr.add(offset + j) != 0 {
                    return &buf[..offset + j + 1];
                }
            }
        }
        // Just more zeros. How boring.
        i -= 32;
    }
    // Last pathetic bytes that didn’t deserve vectorization.
    while i > 0 {
        if *ptr.add(i - 1) != 0 {
            return &buf[..i];
        }
        i -= 1;
    }
    // All zero. Everything. Congratulations on finding pure nothingness.
    &[]
}
