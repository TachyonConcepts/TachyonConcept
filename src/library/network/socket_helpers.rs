use std::os::fd::RawFd;
use libc::{fcntl, socklen_t, O_NONBLOCK};
use tracing::trace;

pub unsafe fn prepare_incoming_socket(client_fd: RawFd)
{
    // Give the socket a 1MB send buffer — because bigger is always better (probably).
    let sndbuf_size: i32 = 1 * 1024 * 1024;
    libc::setsockopt(
        client_fd,
        libc::SOL_SOCKET,
        libc::SO_SNDBUF,
        &sndbuf_size as *const _ as *const libc::c_void,
        size_of::<libc::c_int>() as _,
    );
    // Check if the OS actually listened to us or just pretended to.
    let mut size: libc::c_int = 0;
    let mut len: socklen_t = size_of::<libc::c_int>() as socklen_t;
    libc::getsockopt(
        client_fd,
        libc::SOL_SOCKET,
        libc::SO_SNDBUF,
        &mut size as *mut _ as *mut libc::c_void,
        &mut len,
    );
    trace!("Real sndbuf client size: {} bytes", size);
    // Enable busy polling — let the kernel aggressively wait like it's on caffeine.
    let timeout: i32 = 50;
    libc::setsockopt(
        client_fd,
        libc::SOL_SOCKET,
        libc::SO_BUSY_POLL,
        &timeout as *const _ as *const libc::c_void,
        size_of::<i32>() as socklen_t,
    );
    // Disable Nagle's algorithm — small packets need love too.
    let flag: i32 = 1;
    libc::setsockopt(
        client_fd,
        libc::IPPROTO_TCP,
        libc::TCP_NODELAY,
        &flag as *const _ as *const libc::c_void,
        size_of::<i32>() as socklen_t,
    );
    // Turn on zero-copy because copying memory is so 20th century.
    let enable: libc::c_int = 1;
    libc::setsockopt(
        client_fd,
        libc::SOL_SOCKET,
        libc::SO_ZEROCOPY,
        &enable as *const _ as *const libc::c_void,
        size_of_val(&enable) as _,
    );
    // Make the socket non-blocking — like a true async rebel.
    let flag = fcntl(client_fd, libc::F_GETFL);
    fcntl(client_fd, libc::F_SETFL, flag | O_NONBLOCK);
}