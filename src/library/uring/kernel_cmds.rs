use crate::library::{
    server::BUFFER_SIZE,
    server_internals::{BUF_GROUP, BUFFER_REGISTER_CODE, CODE_ACCEPT},
};
use io_uring::{opcode, squeue, squeue::Flags, types};
use libc::{MSG_DONTWAIT, SOCK_NONBLOCK, msghdr};
use std::os::fd::RawFd;
use tracing::trace;

#[inline(always)]
pub fn provide_buffer(buffer: *mut u8, buffer_id: u16, buffer_size: i32) -> squeue::Entry {
    // Politely hand the kernel a chunk of memory and whisper: "You may feast now"
    trace!("Kernel Call: ProvideBuffer");
    opcode::ProvideBuffers::new(buffer, buffer_size, 1, BUF_GROUP, buffer_id)
        .build()
        .user_data(BUFFER_REGISTER_CODE)
}

#[inline(always)]
pub fn accept_multi(fd: RawFd) -> squeue::Entry {
    // Accept multiple connections in a single syscall, like a bouncer at a very busy nightclub
    trace!("Kernel Call: AcceptMulti");
    opcode::AcceptMulti::new(types::Fd(fd))
        .flags(libc::SOCK_CLOEXEC | SOCK_NONBLOCK)
        .build()
        .user_data(CODE_ACCEPT)
}

#[inline(always)]
pub unsafe fn send_raw_hdr(user_data: u64, client_fd: RawFd, data: &msghdr) -> squeue::Entry {
    // Send an msghdr as-is — no questions asked, no bytes spared.
    trace!("Kernel Call: Send (HDR)");
    opcode::SendMsg::new(types::Fd(client_fd), data)
        .build()
        .user_data(user_data)
        .flags(Flags::SKIP_SUCCESS)
}

#[inline(always)]
pub unsafe fn send(user_data: u64, client_fd: RawFd, data: &[u8], more: bool) -> squeue::Entry {
    // Send bytes like your life depends on it. If `more` is true, tease the TCP stack a bit.
    trace!("Kernel Call: Send");
    trace!("    Write {} bytes", data.len());
    if more {
        opcode::Send::new(types::Fd(client_fd), data.as_ptr(), data.len() as u32)
            .flags(libc::MSG_MORE | MSG_DONTWAIT) // "There's more coming, trust me"
            .build()
            .user_data(user_data)
            .flags(Flags::SKIP_SUCCESS)
    } else {
        opcode::Send::new(types::Fd(client_fd), data.as_ptr(), data.len() as u32)
            .flags(MSG_DONTWAIT)
            .build()
            .user_data(user_data)
            .flags(Flags::SKIP_SUCCESS)
    }
}

#[inline(always)]
pub unsafe fn send_zero_copy(user_data: u64, client_fd: RawFd, data: &[u8]) -> squeue::Entry {
    // Send, but cooler — zero-copy style. Don’t move bytes, just wave at them meaningfully.
    trace!("Kernel Call: SendZc");
    opcode::SendZc::new(types::Fd(client_fd), data.as_ptr(), data.len() as u32)
        .build()
        .user_data(user_data)
}

#[inline(always)]
pub unsafe fn send_msg_zero_copy(user_data: u64, client_fd: RawFd, data: &msghdr) -> squeue::Entry {
    // Same vibe as above, but with a full-blown message header. No copies, no regrets.
    trace!("Kernel Call: SendZc");
    opcode::SendMsgZc::new(types::Fd(client_fd), data)
        .build()
        .user_data(user_data)
        .flags(Flags::SKIP_SUCCESS)
}

#[inline(always)]
pub unsafe fn recv_multi(client_fd: RawFd, user_data: u64) -> squeue::Entry {
    // Receive multiple packets from the kernel buffet. All-you-can-eat mode.
    trace!("Kernel Call: RecvMulti");
    opcode::RecvMulti::new(types::Fd(client_fd), BUF_GROUP)
        .build()
        .user_data(user_data)
}

#[inline(always)]
pub unsafe fn recv_buf_group(client_fd: RawFd, user_data: u64) -> squeue::Entry {
    // Ask the kernel to give you *any* buffer from your magical group of registered ones.
    trace!("Kernel Call: Recv (buf group)");
    opcode::Recv::new(
        types::Fd(client_fd),
        std::ptr::null_mut(), // "You choose the buffer, I trust you"
        BUFFER_SIZE as u32,
    )
    .buf_group(BUF_GROUP)
    .build()
    .user_data(user_data)
    .flags(Flags::BUFFER_SELECT) // Let the kernel do the picking
}

#[inline(always)]
pub unsafe fn recv(client_fd: RawFd, user_data: u64, buffer: *mut u8, len: u32) -> squeue::Entry {
    // Old-school receive into this specific buffer. No surprises. Boring. Safe.
    trace!("Kernel Call: Recv");
    opcode::Recv::new(types::Fd(client_fd), buffer, len)
        .flags(MSG_DONTWAIT)
        .build()
        .user_data(user_data)
}

#[inline(always)]
pub unsafe fn poll_add(client_fd: RawFd, user_data: u64) -> squeue::Entry {
    // Attach a sensor to this fd and scream if anything happens.
    trace!("Kernel Call: PollAddMulti");
    opcode::PollAdd::new(
        types::Fd(client_fd),
        (libc::POLLIN | libc::POLLRDHUP | libc::POLLHUP) as u32,
    )
    .multi(true)
    .build()
    .user_data(user_data)
}

#[inline(always)]
pub unsafe fn async_cancel() -> squeue::Entry {
    // Yeet all async operations. No cleanup. No goodbyes. Just vanish.
    opcode::AsyncCancel::new(0).build()
}
