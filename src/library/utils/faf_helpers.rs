use core::arch::asm;
use tracing::info;

//    Borrowed brilliance alert!
// Massive thanks to @errantmind for the amazing raw syscall and REUSEPORT BPF filter setup!
// You can find the original genius here: https://github.com/errantmind/faf
// I didn’t write this, I just had the good sense to copy it.

#[inline(always)]
pub fn sys_call5(
    mut num: isize,
    arg1: isize,
    arg2: isize,
    arg3: isize,
    arg4: isize,
    arg5: isize,
) -> isize {
    // Direct system call — because libc is for cowards.
    unsafe {
        asm!(
        "syscall",
        in("rax") num,
        in("rdi") arg1,
        in("rsi") arg2,
        in("rdx") arg3,
        in("r10") arg4,
        in("r8") arg5,
        out("rcx") _,
        out("r11") _,
        lateout("rax") num,
        options(nostack, preserves_flags));
        num
    }
}

// Thanks a lot errantmind for cool sock solution!
// Author: https://github.com/errantmind/faf

#[repr(C)]
struct SockFilter {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

#[repr(C)]
struct SockFprog {
    pub len: u16,
    pub filter: *mut SockFilter,
}

pub const SYS_SETSOCKOPT: u32 = 54;
pub const SOL_SOCKET: i32 = 1;
pub const SO_ATTACH_REUSEPORT_CBPF: i32 = 51;

#[inline(always)]
pub fn attach_reuseport_cbpf(fd: isize) {
    // BPF filter to let the kernel distribute connections based on CPU.
    // Because sometimes you just want your sockets load-balanced by fate itself.

    // From the sacred texts of Linux kernel headers:
    const BPF_LD: u16 = 0x00;
    const BPF_RET: u16 = 0x06;

    // BPF_SIZE
    const BPF_W: u16 = 0x00;

    // BPF_MODE
    const BPF_ABS: u16 = 0x20;

    // https://elixir.bootlin.com/linux/latest/source/include/uapi/linux/filter.h
    // BPF_RVAL
    const BPF_A: u16 = 0x10;

    // SKF
    const SKF_AD_OFF: i32 = -0x1000;
    const SKF_AD_CPU: i32 = 36;

    // This filter says: "Load the current CPU ID, then return it as the hash bucket"
    let mut code: [SockFilter; 2] = [
        SockFilter {
            code: BPF_LD | BPF_W | BPF_ABS,
            jt: 0,
            jf: 0,
            k: (SKF_AD_OFF + SKF_AD_CPU) as u32,
        },
        SockFilter {
            code: BPF_RET | BPF_A,
            jt: 0,
            jf: 0,
            k: 0,
        },
    ];

    let prog = SockFprog {
        len: code.len() as u16,
        filter: code.as_mut_ptr(),
    };
    // Raw syscall time. No wrappers. No safety nets. Just steel and fire.
    let ret = sys_call5(
        SYS_SETSOCKOPT as isize,
        fd,
        SOL_SOCKET as isize,
        SO_ATTACH_REUSEPORT_CBPF as isize,
        &prog as *const _ as _,
        size_of::<SockFprog>() as isize,
    );
    // This prints either "it worked" or "you summoned kernel demons"
    info!(
        "SO_ATTACH_REUSEPORT_CBPF ret: {}, size = {}",
        ret,
        size_of::<SockFprog>() as isize
    );
}
