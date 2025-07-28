use std::arch::x86_64::{
    __m256i, _mm256_and_si256, _mm256_cmpeq_epi8,
    _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_set1_epi8,
};
// Welcome to the hot path. This function lives in a tight loop and eats CPU for breakfast.
// Touch it, and the benchmark gods will smite you.
//
// Current benchmark: ~10ns per request.
// Not bad. Not great. But very much "hold my beer".

pub type RequestEntry<'a> = (&'a [u8], &'a [u8]); // (method, path). Everything else is lies.

#[target_feature(enable = "avx2")]
pub unsafe fn parse_one_manual(ptr: *const u8) -> Option<((usize, u8), (usize, u8), usize)> {
    let first8: u64 = *(ptr as *const u64);
    const GET_MASK: u64 = u64::from_le_bytes(*b"GET \0\0\0\0");
    const POST_MASK: u64 = u64::from_le_bytes(*b"POST \0\0\0");
    if first8 & 0xFFFFFFFF == GET_MASK {
        let method_len: u8 = 3;
        let path_start: usize = 4;
        let chunk: __m256i = _mm256_loadu_si256(ptr as *const __m256i);
        let space: __m256i = _mm256_set1_epi8(b' ' as i8);
        let cmp: __m256i = _mm256_cmpeq_epi8(chunk, space);
        let mask: u32 = _mm256_movemask_epi8(cmp) as u32;
        let mask2: u32 = mask >> path_start;
        if mask2 == 0 {
            return None;
        }
        let path_len: u8 = mask2.trailing_zeros() as u8;
        return Some(((0, method_len), (path_start, path_len), 0));
    }
    if first8 & 0xFFFFFFFFFF == POST_MASK {
        let method_len: u8 = 4;
        let path_start: usize = 5;
        let chunk: __m256i = _mm256_loadu_si256(ptr as *const __m256i);
        let space: __m256i = _mm256_set1_epi8(b' ' as i8);
        let cmp: __m256i = _mm256_cmpeq_epi8(chunk, space);
        let mask: u32 = _mm256_movemask_epi8(cmp) as u32;
        let mask2: u32 = mask >> path_start;
        if mask2 == 0 {
            return None;
        }
        let path_len: u8 = mask2.trailing_zeros() as u8;
        return Some(((0, method_len), (path_start, path_len), 0));
    }
    let chunk: __m256i = _mm256_loadu_si256(ptr as *const __m256i);
    let space: __m256i = _mm256_set1_epi8(b' ' as i8);
    let cmp: __m256i = _mm256_cmpeq_epi8(chunk, space);
    let mask: u32 = _mm256_movemask_epi8(cmp) as u32;
    if mask == 0 {
        return None;
    }
    let method_len: u8 = mask.trailing_zeros() as u8;
    let path_start: usize = method_len as usize + 1;
    let mask2: u32 = mask >> path_start;
    if mask2 == 0 {
        return None;
    }
    let path_len: u8 = mask2.trailing_zeros() as u8;
    Some(((0, method_len), (path_start, path_len), 0))
}

#[target_feature(enable = "avx2")]
pub unsafe fn find_request_starts_avx2<const N: usize>(buf: &[u8]) -> ([usize; N], usize) {
    let mut found: [usize; N] = [0usize; N];
    let mut count: usize = 0;
    let len: usize = buf.len();
    let mut i: usize = 0;
    while i + 66 <= len && count < N {
        let ptr: *const u8 = buf.as_ptr();
        let c0a: __m256i = _mm256_loadu_si256(ptr.add(i) as *const __m256i);
        let c1a: __m256i = _mm256_loadu_si256(ptr.add(i + 1) as *const __m256i);
        let c2a: __m256i = _mm256_loadu_si256(ptr.add(i + 2) as *const __m256i);

        let get_mask_a: u32 = _mm256_movemask_epi8(_mm256_and_si256(
            _mm256_cmpeq_epi8(c0a, _mm256_set1_epi8(b'G' as i8)),
            _mm256_and_si256(
                _mm256_cmpeq_epi8(c1a, _mm256_set1_epi8(b'E' as i8)),
                _mm256_cmpeq_epi8(c2a, _mm256_set1_epi8(b'T' as i8)),
            ),
        )) as u32;
        let post_mask_a: u32 = _mm256_movemask_epi8(_mm256_and_si256(
            _mm256_cmpeq_epi8(c0a, _mm256_set1_epi8(b'P' as i8)),
            _mm256_and_si256(
                _mm256_cmpeq_epi8(c1a, _mm256_set1_epi8(b'O' as i8)),
                _mm256_cmpeq_epi8(c2a, _mm256_set1_epi8(b'S' as i8)),
            ),
        )) as u32;
        let c0b: __m256i = _mm256_loadu_si256(ptr.add(i + 32) as *const __m256i);
        let c1b: __m256i = _mm256_loadu_si256(ptr.add(i + 33) as *const __m256i);
        let c2b: __m256i = _mm256_loadu_si256(ptr.add(i + 34) as *const __m256i);
        let get_mask_b: u32 = _mm256_movemask_epi8(_mm256_and_si256(
            _mm256_cmpeq_epi8(c0b, _mm256_set1_epi8(b'G' as i8)),
            _mm256_and_si256(
                _mm256_cmpeq_epi8(c1b, _mm256_set1_epi8(b'E' as i8)),
                _mm256_cmpeq_epi8(c2b, _mm256_set1_epi8(b'T' as i8)),
            ),
        )) as u32;
        let post_mask_b: u32 = _mm256_movemask_epi8(_mm256_and_si256(
            _mm256_cmpeq_epi8(c0b, _mm256_set1_epi8(b'P' as i8)),
            _mm256_and_si256(
                _mm256_cmpeq_epi8(c1b, _mm256_set1_epi8(b'O' as i8)),
                _mm256_cmpeq_epi8(c2b, _mm256_set1_epi8(b'S' as i8)),
            ),
        )) as u32;
        let mask_a: u32 = get_mask_a | post_mask_a;
        let mask_b: u32 = get_mask_b | post_mask_b;
        if mask_a != 0 {
            let mut bits: u32 = mask_a;
            while bits != 0 && count < N {
                let tz: usize = bits.trailing_zeros() as usize;
                found[count] = i + tz;
                count += 1;
                bits &= bits - 1;
            }
        }
        if mask_b != 0 {
            let mut bits: u32 = mask_b;
            while bits != 0 && count < N {
                let tz: usize = bits.trailing_zeros() as usize;
                found[count] = i + 32 + tz;
                count += 1;
                bits &= bits - 1;
            }
        }
        i += 64;
    }
    while i + 34 <= len && count < N {
        let p0: *const __m256i = buf.as_ptr().add(i) as *const __m256i;
        let p1: *const __m256i = buf.as_ptr().add(i + 1) as *const __m256i;
        let p2: *const __m256i = buf.as_ptr().add(i + 2) as *const __m256i;

        let chunk0: __m256i = _mm256_loadu_si256(p0);
        let chunk1: __m256i = _mm256_loadu_si256(p1);
        let chunk2: __m256i = _mm256_loadu_si256(p2);
        let get_mask: u32 = _mm256_movemask_epi8(_mm256_and_si256(
            _mm256_cmpeq_epi8(chunk0, _mm256_set1_epi8(b'G' as i8)),
            _mm256_and_si256(
                _mm256_cmpeq_epi8(chunk1, _mm256_set1_epi8(b'E' as i8)),
                _mm256_cmpeq_epi8(chunk2, _mm256_set1_epi8(b'T' as i8)),
            ),
        )) as u32;
        let post_mask: u32 = _mm256_movemask_epi8(_mm256_and_si256(
            _mm256_cmpeq_epi8(chunk0, _mm256_set1_epi8(b'P' as i8)),
            _mm256_and_si256(
                _mm256_cmpeq_epi8(chunk1, _mm256_set1_epi8(b'O' as i8)),
                _mm256_cmpeq_epi8(chunk2, _mm256_set1_epi8(b'S' as i8)),
            ),
        )) as u32;
        let mask: u32 = get_mask | post_mask;
        if mask != 0 {
            let mut bits: u32 = mask;
            while bits != 0 && count < N {
                let tz: usize = bits.trailing_zeros() as usize;
                found[count] = i + tz;
                count += 1;
                bits &= bits - 1;
            }
        }
        i += 32;
    }
    (found, count)
}
#[derive(Copy, Clone, Default, Debug)]
pub struct RequestRawEntry {
    pub method_start: usize,
    pub method_len: u8,
    pub path_start: usize,
    pub path_len: u8,
}

#[inline(always)]
pub unsafe fn parse_http_methods_paths<'a, const N: usize>(
    buf: &'a [u8],
) -> ([RequestEntry<'a>; N], usize) {
    let mut out: [RequestEntry<'a>; N] = [(&[][..], &[][..]); N];
    let mut count: usize = 0;
    let (starts, total): ([usize; N], usize) = find_request_starts_avx2::<N>(buf);

    for i in 0..total {
        let start: usize = starts[i];
        if start + 32 > buf.len() {
            break;
        }
        let raw: *const u8 = buf.as_ptr().add(start);
        if let Some(((m_off, m_len), (p_off, p_len), _adv)) = parse_one_manual(raw) {
            let m_ptr = buf.as_ptr().add(start + m_off);
            let p_ptr = buf.as_ptr().add(start + p_off);
            out[count] = (
                core::slice::from_raw_parts(m_ptr, m_len as usize),
                core::slice::from_raw_parts(p_ptr, p_len as usize),
            );
            count += 1;
        }
    }

    (out, count)
}
