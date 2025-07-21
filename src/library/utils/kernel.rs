use crate::library::server::BUFFER_SIZE;
use libc::{LOG_EMERG, LOG_USER, PR_SET_NAME, closelog, openlog, prctl, syslog};
use std::arch::asm;
use std::ffi::CString;
use std::fs::OpenOptions;
use std::io::Write;
use tracing::error;

// Hot zone detected. Abandon safety, ye who enter here.
//
// This is a full-on unsafe, no_mangle, extern "C" AVX-based memory heist.
// It copies a fixed-size kernel-owned buffer into `dst` using unaligned vector loads/stores.
//
// Notes:
// - BUFFER_SIZE is assumed to be 6272 bytes
// - Each iteration copies 32 bytes using `vmovdqu` (so we do 196 iterations)
// - We don’t care about alignment, borrow checker, or safety. Only speed and fire.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_kernel_owned_buffer(
    index: usize,                      // Which buffer to extract from the pool
    buffers: *const [u8; BUFFER_SIZE], // Pointer to big slab of buffers
    dst: *mut u8,                      // Destination to yeet the data into
) {
    asm!(
        // Calculate offset: rcx = index * 6272
        "imul rcx, rdi, 6272", // Because multiplying by BUFFER_SIZE manually is faster than thinking
        // r10 = &buffers[index]
        "lea r10, [rsi + rcx]", // Just casually pointer-walking in raw AVX land
        "xor r8, r8", // r8 = offset counter (from 0 to 6272)
        "2:", // Label: start copy loop
        "cmp r8, 6272", // Have we copied it all?
        "jge 3f", // If yes, jump to done
        // ymm0 = *(r10 + r8)
        "vmovdqu ymm0, [r10 + r8]", // Load 32 bytes from source (unaligned, because life is too short to align)
        "vmovdqu [rdx + r8], ymm0", // Store 32 bytes to destination
        "add r8, 32", // Move to next chunk
        "jmp 2b", // Repeat until done
        "3:", // Label: done
        // Clean up AVX state because AVX-512 people get cranky
        "vzeroupper", // Avoid transition penalty into legacy SSE code. Intel says this matters. We believe them.
        in("rdi") index, // buffer index
        in("rsi") buffers, // &buffers
        in("rdx") dst,  // destination pointer
        out("rcx") _, // Clobbered: offset multiplier
        out("r10") _, // Clobbered: actual buffer pointer
        out("r8") _, // Clobbered: loop counter
        out("ymm0") _,  // Clobbered: temp vector register
        options(nostack, preserves_flags), // We solemnly swear not to mess with the stack
    );
}

pub unsafe fn log_kernel_error(message: &str, code: &str) {
    openlog(std::ptr::null(), 0, LOG_USER);
    syslog(
        LOG_EMERG,
        b"%s\0".as_ptr() as *const i8,
        message.as_ptr() as *const i8,
    );
    closelog();

    if let Ok(mut file) = OpenOptions::new().write(true).open("/dev/kmsg") {
        let _ = writeln!(file, "<3>Tachyon: {}", message); // <3> = error priority
    }
    let code = CString::new(code).unwrap();
    prctl(PR_SET_NAME, code.as_ptr());

    error!("Tachyon has entered HELL mode: realtime + ub_dma = APOCALYPSE");
    error!("Doomguy not found. System doomed.");
    error!("System breached. Demons unleashed. Closing portal...");

    let ptr = 0xDEAD as *mut u8;
    *ptr = 0x42;

    libc::abort()
}
