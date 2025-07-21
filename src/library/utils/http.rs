use memchr::memchr;
use memchr::memmem::Finder;
use std::arch::x86_64::{
    __m256i, _MM_HINT_NTA, _blsr_u32, _mm_prefetch, _mm256_cmpeq_epi8, _mm256_loadu_si256,
    _mm256_movemask_epi8, _mm256_set1_epi8, _tzcnt_u32,
};
// Welcome to the hot path. This function lives in a tight loop and eats CPU for breakfast.
// Touch it, and the benchmark gods will smite you.
//
// Current benchmark: ~435ns per iteration with ~40 HTTP pipelined requests.
// Not bad. Not great. But very much "hold my beer".

pub type RequestEntry<'a> = (&'a [u8], &'a [u8]); // (method, path). Everything else is lies.

#[inline(always)]
pub fn parse_http_methods_paths<const N: usize>(buf: &[u8]) -> ([RequestEntry<'_>; N], usize) {
    let mut out: [RequestEntry; N] = [(&[][..], &[][..]); N];
    let mut n = 0;
    // Fast searcher for "\r\n\r\n" — the HTTP handshake of "I'm done talking".
    let finder = Finder::new(b"\r\n\r\n");
    let mut start = 0;
    // Loop through pipelined HTTP requests, like a well-oiled assembly line.
    // We stop at N, because infinite loops are only fun for the kernel.
    for end in finder.find_iter(buf).take(N) {
        let header = &buf[start..end]; // The sacred scroll: full HTTP header block
        start = end + 4; // Skip past "\r\n\r\n" like a real adult
        // Find the first and second spaces in the request line.
        // We're looking for "METHOD SP PATH SP ..." — not "poetry SP in SP motion".
        if let Some(sp1) = memchr(b' ', header) {
            // Yes, this offset dance is safe and elegant.
            // It’s also one of the reasons we don’t let interns write hot-path code.
            if let Some(sp2) = memchr(b' ', &header[sp1 + 1..]) {
                out[n] = (
                    &header[..sp1],                  // METHOD (e.g., "GET", "POST", "DELETE", "BREW")
                    &header[sp1 + 1..sp1 + 1 + sp2], // PATH (e.g., "/plaintext")
                );
                n += 1;
            }
        }
    }
    // Return what we found. Ignore the rest. Like a good hacker at a bad party.
    (out, n)
}

/// This is the *unsafe, unhinged, and unapologetically fast* version.
///
/// It doesn't parse HTTP — it **rips** `(METHOD, PATH)` out of pipelined requests
/// using AVX2, raw pointers, and zero regard for correctness in edge cases.
///
/// Assumptions (read: comforting lies we tell ourselves):
/// - Input is valid ASCII-only HTTP.
/// - Every request ends with `\r\n\r\n`. Always. No exceptions. Trust me, bro.
/// - No chunked encoding. No meaningful headers. No tears.
/// - METHOD and PATH are space-separated. Everything after that is dead to us.
///
/// Do not use this in production unless your idea of fun includes:
/// - Memory safety bugs
/// - Invalid HTTP causing very real business consequences
/// - Explaining AVX2 stack traces to your boss at 2 AM
///
/// Use with caution. Or better yet: don't use it.
/// Benchmark it, brag about it, and quietly walk away.

#[inline(always)]
pub unsafe fn unreliable_parse_http_methods_paths<const N: usize>(
    buf: &[u8],
) -> ([RequestEntry<'_>; N], usize) {
    // Output array for (method, path) pairs – totally unreliable, just blazing fast.
    let mut out: [RequestEntry<'_>; N] = [(&[][..], &[][..]); N];
    let ptr = buf.as_ptr();
    let len = buf.len();

    let mut i = 0usize; // Input cursor
    let mut n = 0usize; // Parsed request count
    let mut start = 0usize; // Start of the current request header

    // AVX2 constants for finding '\n' and ' '
    let v_lf = _mm256_set1_epi8(b'\n' as i8); // Line feed
    let v_sp = _mm256_set1_epi8(b' ' as i8); // Space

    // After weeks of pain, trial, profiling, and existential crisis,
    // this overlap‑sliding window of 192 bytes, shifted by 128, gave the best tradeoff:
    // ~80–114 ns per pass with up to 15/20 requests captured. Acceptable losses. We sleep now.
    while i + 192 <= len && n < N {
        // Load 3x 64-byte chunks with overlap for better double-CRLF detection
        let chunk1 = _mm256_loadu_si256(ptr.add(i) as *const __m256i);
        let chunk2 = _mm256_loadu_si256(ptr.add(i + 64) as *const __m256i);
        let chunk3 = _mm256_loadu_si256(ptr.add(i + 128) as *const __m256i);
        // A nod to the gods of cache
        _mm_prefetch(ptr.add(i + 256) as *const i8, _MM_HINT_NTA); // Prefetch next window
        // Create bitmasks for LF positions in each chunk
        let mut mask1 = _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk1, v_lf)) as u32;
        let mut mask2 = _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk2, v_lf)) as u32;
        let mut mask3 = _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk3, v_lf)) as u32;
        // Macro to process each LF mask – we look for "\r\n\r\n" ending the header,
        // then extract METHOD and PATH up to 64 bytes max using space positions.
        macro_rules! process_mask {
            ($mask:ident, $offset:expr) => {
                while $mask != 0 && n < N {
                    let tz = $mask.trailing_zeros() as usize;
                    let pos = i + $offset + tz;
                    $mask = _blsr_u32($mask); // Clear the lowest bit (next match)

                    if pos >= 3 && pos + 1 < len {
                        // Check if bytes before this LF form the magic "\r\n\r\n" sequence
                        let word = *(ptr.add(pos - 3) as *const u32);
                        if word == 0x0a0d0a0d {
                            // This is the end of the HTTP header
                            let hdr_ptr = ptr.add(start);
                            let hdr_len = pos - 1 - start;
                            let limit = hdr_len.min(64);
                            // Scan for METHOD SP PATH SP ...
                            let chunk = _mm256_loadu_si256(hdr_ptr as *const __m256i);
                            let space_mask =
                                _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, v_sp)) as u32;

                            let s1 = _tzcnt_u32(space_mask) as usize;
                            let s2 = _tzcnt_u32(_blsr_u32(space_mask)) as usize;
                            // Make sure we don’t create nonsense slices
                            if s1 < limit && s2 < limit && s1 + 1 < s2 {
                                out[n] = (
                                    core::slice::from_raw_parts(hdr_ptr, s1),
                                    core::slice::from_raw_parts(hdr_ptr.add(s1 + 1), s2 - s1 - 1),
                                );
                                n += 1;
                            }
                            // Move start pointer to just after "\r\n\r\n"
                            start = pos + 1;
                        }
                    }
                }
            };
        }
        // Process all 3 masks – this block alone defeated multiple AVX2 optimizations and overlap schemes
        process_mask!(mask1, 0);
        process_mask!(mask2, 64);
        process_mask!(mask3, 128);
        i += 128; // Overlap by 64 bytes – we tried 64/96/128/192 steps. This won.
    }
    // Output the partially reliable results
    (out, n)
}
