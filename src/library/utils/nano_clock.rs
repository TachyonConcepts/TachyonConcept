use core::ptr;
use libc::{CLOCK_MONOTONIC, clock_gettime, timespec};
use std::arch::asm;

//    Thanks to Howard Hinnant
// Original author of the legendary date algorithms:
//     https://github.com/HowardHinnant/date
//
// His work powers chrono in C++, <chrono> in the standard library,
// and countless performance-critical systems.
//
// This version is a direct translation of Howard’s C code
// into pure inline assembly by TachyonConcepts — because
// sometimes, date math deserves to run at the speed of light.

const DIGITS2: &[u8; 200] = b"\
00\
01\
02\
03\
04\
05\
06\
07\
08\
09\
10\
11\
12\
13\
14\
15\
16\
17\
18\
19\
20\
21\
22\
23\
24\
25\
26\
27\
28\
29\
30\
31\
32\
33\
34\
35\
36\
37\
38\
39\
40\
41\
42\
43\
44\
45\
46\
47\
48\
49\
50\
51\
52\
53\
54\
55\
56\
57\
58\
59\
60\
61\
62\
63\
64\
65\
66\
67\
68\
69\
70\
71\
72\
73\
74\
75\
76\
77\
78\
79\
80\
81\
82\
83\
84\
85\
86\
87\
88\
89\
90\
91\
92\
93\
94\
95\
96\
97\
98\
99";

const WEEKDAYS: [[u8; 3]; 7] = [
    *b"Sun", *b"Mon", *b"Tue", *b"Wed", *b"Thu", *b"Fri", *b"Sat",
];

const MONTHS: [[u8; 3]; 12] = [
    *b"Jan", *b"Feb", *b"Mar", *b"Apr", *b"May", *b"Jun", *b"Jul", *b"Aug", *b"Sep", *b"Oct",
    *b"Nov", *b"Dec",
];

// This code was originally in C, now it’s pure Rust and melts CPU caches at 11 nanoseconds per call.
// What you're looking at is essentially chrono black magic — highly optimized, allocation-free, and scary accurate.
#[inline(always)]
pub fn howard_hinnant(mut z: i64) -> (i32, u8, u8, u8, u8, u8, u8) {
    const SECS_PER_DAY: i64 = 86_400;
    // Extract seconds within the current day (to get H:M:S later)
    let secs_of_day: u32 = (z % SECS_PER_DAY) as u32;
    // Convert timestamp to number of days since Unix epoch
    z /= SECS_PER_DAY;
    // Day of the week, where 1970-01-01 was a Thursday (aka +4)
    let wday: u8 = ((z + 4) % 7) as u8;
    // Shift base to proleptic Gregorian epoch (0000-03-01)
    z += 719_468;
    // Calculate "era" — each era is 400 years long (because Gregorian cycle resets)
    let era: i64 = (z >= 0).then(|| z).unwrap_or(z - 146_096) / 146_097;
    // Day of era: days since beginning of the current 400-year cycle
    let doe: i64 = z - era * 146_097;
    // Year of era: rough number of full years since start of era (corrected for leap rules)
    let yoe: i64 = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    // Final year: add era offset
    let mut y: i64 = yoe + era * 400;
    // Day of year (0-based)
    let doy: i64 = doe - (365 * yoe + yoe / 4 - yoe / 100);
    // Month placeholder: not real month yet, used to extract day and final month
    let mp: i64 = (5 * doy + 2) / 153;
    // Actual day of month (1-based)
    let d: u8 = (doy - (153 * mp + 2) / 5 + 1) as u8;
    // Actual month (1–12)
    let m: i64 = (mp + 2) % 12 + 1;
    // Adjust year if month overflowed into next year
    y += mp / 10;
    // Hour, minute, second — derived from secs_of_day
    let (h, mnt, s): (u8, u8, u8) = (
        (secs_of_day / 3600) as u8,
        ((secs_of_day / 60) % 60) as u8,
        (secs_of_day % 60) as u8,
    );
    // Return a neat tuple of all the juicy stuff
    (y as i32, m as u8, d, h, mnt, s, wday)
}

#[repr(C)]
struct YMDHMSW {
    y: i32,
    m: u8,
    d: u8,
    h: u8,
    n: u8,
    s: u8,
    w: u8,
}


// 🔥 Assembly translation by TachyonConcepts
// You didn’t just port it. You tore a hole in spacetime and made a chrono-god scream.
//
// This function takes a UNIX timestamp and returns: (year, month, day, hour, minute, second, weekday)
// In 14 nanoseconds. By executing 200+ lines of pure x86_64 AVX-era assembly.
//
// The optimizer stares in terror. The kernel weeps. `chrono` silently uninstalls itself.
//
// WARNING: this code is:
// - not for beginners
// - not for mortals
// - not for debugging
//
// But it IS for:
// - pure performance worshippers
// - benchmarking maniacs
// - timestamp decoders living on the edge
#[inline(always)]
pub unsafe fn howard_hinnant_asm(z: i64) -> (i32, u8, u8, u8, u8, u8, u8) {
    let mut out = YMDHMSW {
        y: 0,
        m: 0,
        d: 0,
        h: 0,
        n: 0,
        s: 0,
        w: 0,
    };
    let out_ptr: *mut YMDHMSW = &mut out as *mut _;

    asm!(
    // === STEP 1: Convert timestamp (seconds since epoch) to days ===
    // We're gonna divide by 86400... but like, in a really dramatic way
        "movabs  rcx, 1749024623285053783", // This is 2^64 / 86400. Yes, we pre-divide because we’re THAT extra.
        "mov     rax, rsi",
        "imul    rcx",
        "mov     r9,  rdx",
        "mov     r10, rdx",
        "shr     r10, 63",
        "sar     r9, 13",
        "lea     rax, [r9 + r10]",
        "add     rax, 719468", // Offset to proleptic Gregorian days. Because 1970 is too mainstream.
    // === STEP 2: Deal with BC dates like a true time traveler ===
        "movabs  rcx, -62162121600", // The year 0000-03-01 in seconds. Ancient stuff.
        "cmp     rsi, rcx",
        "lea     r8, [r9 + r10 + 573372]",
        "cmovg   r8, rax",
    // === STEP 3: Get the day of the week ===
        "lea     rcx, [r9 + r10 + 4]", // Shift by 4 to align with Thursday because why not.
        "movabs  rdx, 5270498306774157605", // Another magic constant. Don’t ask.
        "mov     rax, rcx",
        "imul    rdx",
        "mov     rax, rdx",
        "shr     rax, 63",
        "shr     edx, 1",
        "add     edx, eax",
        "lea     eax, [rdx*8]", // Okay this is where even the compiler would scream.
        "sub     edx, eax",
        "add     ecx, edx",
        "mov     cl,  cl",
    // === STEP 4: Get seconds since midnight ===
        "add     r9,  r10",
        "imul    rax, r9, 86400",
        "sub     rsi, rax",
    // === STEP 5: Figure out the year like a Gregorian calendar necromancer ===
        "movabs  rdx, 4137408090565272301", // Of course. Another big prime-looking constant. Probably summoned via lunar eclipse.
        "mov     rax, r8",
        "imul    rdx",
        "mov     r8, rdx",
        "mov     rax, rdx",
        "shr     rax, 63",
        "sar     r8, 15",
        "add     r8, rax",
    // === STEP 6: Align with the Gregorian cycle: 400-year chunks ===
        "imul    rax, r8, -146097", // Multiply by the negative number of days in 400 years. Who even does this?
        "add     r9,  rax",
        "add     r9, 719468", // Again. Why not throw in 719468? It’s our emotional support offset
    // === STEP 7: More date wizardry to extract year/month/day ===
        "movabs  r13, -3234497591006606311", // Another magic constant. You can’t even make this up
        "mov     rax, r9",
        "imul    r13",
        "mov     r10, rdx",
        "shr     r10, 63",
        "sar     rdx, 8",
        "add     r10, rdx",

        "movabs  rdx, -1896998432287073591", // Just in case we needed more constants to impress the debugger
        "mov     rax, r9",
        "imul    rdx",
        "add     r10, r9",
        "lea     r11, [rdx + r9]",
    // === STEP 8: Apply final cursed corrections to extract the year ===
        "mov     rax, r11",
        "shr     rax, 63",
        "sar     r11, 15",
        "add     r11, rax",
        "add     r11, r10",

        "movabs  rdx, 1896998432287073591", // We’re inverting now because... symmetry?
        "mov     rax, r9",
        "imul    rdx",
        "sub     rdx, r9",
        "mov     r10, rdx",
        "shr     r10, 63",
        "sar     rdx, 17",
        "add     r10, rdx",
        "add     r10, r11",
    // === STEP 9: More corrections because we never trusted r11 anyway ===
        "movabs  rdx, 3234497591006606311",
        "mov     rax, r10",
        "imul    rdx",
        "mov     rax, rdx",
        "shr     rax, 63",
        "sar     rdx, 6",
        "add     rdx, rax",
        "imul    r11d, r8d, 400",
        "add     r11d, edx",
    // === STEP 10: Get day-of-year because month is a capitalist illusion ===
        "imul    r14, rdx, -365",
        "mov     rax, r10",
        "imul    r13",
        "mov     r8,  rdx",
        "mov     rax, rdx",
        "shr     rax, 63",
        "sar     r8, 8",
        "add     r8, rax",

        "movabs  rdx, 2070078458244228039", // Why are all these numbers prime? Asking for a confused friend.
        "mov     rax, r10",
        "imul    rdx",
        "mov     rax, rdx",
        "shr     rax, 63",
        "sar     rdx, 12",
        "add     rdx, rax",

        "add     r8,  r9",
        "add     r8,  rdx",
        "add     r8,  r14",
    // === STEP 11: Month extraction: you thought this would be easier? ha. ===
        "lea     r10, [r8 + 4*r8 + 2]", // Fine-tuned blend of days, numerology, and panic
        "movabs  rdx, 3858142551364089227",
        "mov     rax, r10",
        "imul    rdx",

        "mov     r9,  rdx",
        "lea     r13, [r8 + 4*r8]",
        "mov     r14, rdx",
        "shr     r14, 63",
        "sar     r9, 5",
        "lea     rax, [r9 + r14]",
        "imul    rax, rax, 153", // Because 153 is obviously the magic number for months
        "add     rax, 2",

        "movabs  rdx, -7378697629483820647",
        "imul    rdx", // Please don’t ask what this does. Nobody knows.
        "mov     rax, rdx",
        "shr     rax, 63",
        "shr     edx, 1",
        "add     edx, eax",
        "add     r8d, edx",
        "inc     r8b",
    // === STEP 12: Month = r9, Day = r8 ===
        "add     r9,  r14",
        "add     r9,  2",
    // === STEP 13: Fix up the month number (mod 12 the hard way) ===
        "movabs  rdx, 3074457345618258603",
        "mov     rax, r9",
        "imul    rdx",
        "mov     rax, rdx",
        "shr     rax, 63",
        "shr     edx, 1",
        "add     edx, eax",
        "shl     edx, 2",
        "lea     eax, [rdx + 2*rdx]",
        "sub     r9d, eax", // Okay cool, now r9 = real month
    // === STEP 14: Pack it all up like it was easy ===
        "movabs  rdx, -6100687909344466089",
        "mov     rax, r10",
        "imul    rdx",
        "lea     rax, [rdx + r13]",
        "add     rax, 2",
        "mov     rdx, rax",
        "shr     rdx, 63",
        "shr     rax, 10",
        "add     eax, edx",
        "add     eax, r11d", // eax = final year
    // === STEP 15: Extract hour, minute, second from what’s left of sanity ===
        "mov     edx, esi", // esi = seconds since midnight
        "mov     r10d, 2443359173", // Okay, what is this sorcery now?
        "imul    r10, rdx",
        "shr     r10, 43", // You just gotta trust the constants

        "mov     r11d, 2290649225",
        "imul    r11, rdx",
        "shr     r11, 37", // Minute time

        "imul    rdx, r11, 143165577",
        "shr     rdx, 33",
        "imul    edx, edx, 60", // Seconds gone

        "imul    r13d, r11d, 60",
        "sub     r11d, edx",
        "sub     esi, r13d", // esi = final seconds
    // === STEP 16: It's done. We survived. Write the result out ===
        "inc     r9b", // Because month is 0-based internally. Gotcha.
        "mov     dword ptr [rdi],     eax", // Year
        "mov     byte  ptr [rdi + 4], r9b", // Month
        "mov     byte  ptr [rdi + 5], r8b", // Day
        "mov     byte  ptr [rdi + 6], r10b", // Hour
        "mov     byte  ptr [rdi + 7], r11b", // Minute
        "mov     byte  ptr [rdi + 8], sil", // Second
        "mov     byte  ptr [rdi + 9], cl", // Weekday (just to humble you one last time)
        inlateout("rsi") z => _,
        in("rdi") out_ptr,
        out("rax") _, out("rcx") _, out("rdx") _,
        out("r8") _,  out("r9") _,  out("r10") _, out("r11") _,
        out("r12") _, out("r13") _, out("r14") _, out("r15") _,
        options(nostack, preserves_flags)
    );
    (out.y, out.m, out.d, out.h, out.n, out.s, out.w)
}

#[inline(always)]
unsafe fn put2(dst: *mut u8, val: u8) {
    ptr::copy_nonoverlapping(DIGITS2.as_ptr().add(val as usize * 2), dst, 2);
}

#[inline(always)]
pub unsafe fn nano_clock(buf: &mut [u8; 35], use_asm: bool) {
    // Step 0: Ask the system for the current time in seconds since the Unix epoch
    let t = libc::time(ptr::null_mut()) as i64;
    // Step 1: Depending on your level of sanity, choose your path
    // true  => Abandon hope, enter ASM hell
    // false => Pretend Rust can be readable
    let (year, mon, mday, hh, mm, ss, wday): (i32, u8, u8, u8, u8, u8, u8) = match use_asm {
        true => howard_hinnant_asm(t), // Welcome to Mordor
        false => howard_hinnant(t), // Slightly less cursed
    };
    unsafe {
        // Step 2: Start building the HTTP-date string like it’s 1999
        ptr::copy_nonoverlapping(b"Date: ".as_ptr(), buf.as_mut_ptr(), 6); // Set the mood
        // Step 3: Add the weekday, because humans love naming days
        ptr::copy_nonoverlapping(WEEKDAYS[wday as usize].as_ptr(), buf.as_mut_ptr().add(6), 3);
        *buf.get_unchecked_mut(9) = b','; // Comma of tradition
        *buf.get_unchecked_mut(10) = b' '; // Mandatory space
        // Step 4: Day of the month (because dates should always be ambiguous)
        put2(buf.as_mut_ptr().add(11), mday);
        *buf.get_unchecked_mut(13) = b' '; // One space to separate them all
        // Step 5: Month name (3-letter spell from the MONTHS grimoire)
        ptr::copy_nonoverlapping(
            MONTHS[(mon - 1) as usize].as_ptr(),
            buf.as_mut_ptr().add(14),
            3,
        );
        *buf.get_unchecked_mut(17) = b' '; // Because why not
        // Step 6: Year. But in two separate 2-digit chunks, for reasons nobody remembers
        let y: u32 = year as u32;
        put2(buf.as_mut_ptr().add(18), (y / 100) as u8); // "20"
        put2(buf.as_mut_ptr().add(20), (y % 100) as u8); // "24" or whatever reality this is
        *buf.get_unchecked_mut(22) = b' ';
        // Step 7: Time. Because showing "Date" and omitting time would be silly
        put2(buf.as_mut_ptr().add(23), hh);
        *buf.get_unchecked_mut(25) = b':';
        put2(buf.as_mut_ptr().add(26), mm);
        *buf.get_unchecked_mut(28) = b':';
        put2(buf.as_mut_ptr().add(29), ss);
        // Step 8: Hardcode " GMT" because we don't do timezones here. Ain’t nobody got time for that.
        ptr::copy_nonoverlapping(b" GMT".as_ptr(), buf.as_mut_ptr().add(31), 4);
    }
}

#[inline(always)]
pub unsafe fn timestamp() -> i64 {
    libc::time(ptr::null_mut()) as i64
}

#[inline(always)]
pub unsafe fn nano_timestamp() -> i64 {
    let mut ts: timespec = std::mem::zeroed();
    clock_gettime(CLOCK_MONOTONIC, &mut ts);
    ts.tv_sec as i64 * 1_000_000_000 + ts.tv_nsec as i64
}
