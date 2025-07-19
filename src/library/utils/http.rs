use memchr::memmem::Finder;

// Welcome to the hot path. This function lives in a tight loop and eats CPU for breakfast.
// Touch it, and the benchmark gods will smite you.
//
// Current benchmark: ~522ns per iteration with ~40 HTTP pipelined requests.
// Not bad. Not great. But very much "hold my beer".

pub type RequestEntry<'a> = (&'a [u8], &'a [u8]); // (method, path). Everything else is lies.

thread_local! {
    // Finder for "\r\n\r\n" – because HTTP can't commit to one newline like a normal protocol.
    static FINDER: Finder<'static> = Finder::new(b"\r\n\r\n");
}

#[inline(always)]
pub fn parse_http_methods_paths<const N: usize>(
    mut buffer: &'_ [u8],
) -> ([RequestEntry<'_>; N], usize) {
    // Preallocate an array of (method, path) tuples. The rest of HTTP? Ignored.
    let mut requests: [RequestEntry; N] = [(&[][..], &[][..]); N];
    let mut count: usize = 0;
    while count < N {
        // Find the end of the HTTP request header (the infamous double CRLF).
        let end: usize = match FINDER.with(|f| f.find(buffer)) {
            Some(i) => i,
            None => break, // No more headers? No more parsing. Bail out.
        };
        // Find end of the request line (\r). If not found, pretend the whole thing is the line.
        let line_end: usize = memchr::memchr(b'\r', &buffer[..end]).unwrap_or(end);
        let line: &[u8] = &buffer[..line_end];
        // First space = end of method. If not found, the request is garbage and we skip it.
        let space1: Option<usize> = memchr::memchr(b' ', line);
        if space1.is_none() {
            buffer = &buffer[end + 4..];
            continue;
        }
        let space1: usize = space1.unwrap();
        let method: &[u8] = &line[..space1];
        // Everything after first space. Expecting: PATH + another space + protocol
        let rest: &[u8] = &line[space1 + 1..];
        let space2: Option<usize> = memchr::memchr(b' ', rest);
        if space2.is_none() {
            buffer = &buffer[end + 4..];
            continue;
        }
        let space2: usize = space2.unwrap();
        let path: &[u8] = &rest[..space2];
        // Store the parsed request in the array.
        requests[count] = (method, path);
        count += 1;
        // Move the buffer past the end of this request.
        buffer = &buffer[end + 4..];
    }
    // Return only what we parsed successfully, and no more.
    (requests, count)
}