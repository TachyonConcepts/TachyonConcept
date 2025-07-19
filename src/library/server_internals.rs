use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::{
    io,
    net::TcpListener
    ,
};
use crate::library::uring::Uring;

pub const BUF_GROUP: u16 = 42;
pub const REQ_RESP_OFFSET: u64 = u64::MAX / 2;
pub const BUFFER_REGISTER_CODE: u64 = 0xFAB;
pub const INIT_REQUEST: u16 = 0xCCA;
pub const POLL_EVENT: u16 = 0xAAA;
pub const CODE_ACCEPT: u64 = 0xA;

#[derive(Debug, Clone, Copy)]
pub struct UserData {
    pub client_id: u32,
    pub buffer_id: u16,
    pub uniq_id: u16,
}

impl UserData {
    #[inline(always)]
    pub const fn pack_user_data(&self) -> u64 {
        ((self.uniq_id as u64) << 48)
            | ((self.buffer_id as u64) << 32)
            | (self.client_id as u64) + REQ_RESP_OFFSET
    }
    pub fn unpack_user_data(user_data: u64) -> Self {
        let raw = user_data - REQ_RESP_OFFSET;
        Self {
            client_id: (raw & 0xFFFF_FFFF) as u32,
            buffer_id: ((raw >> 32) & 0xFFFF) as u16,
            uniq_id: ((raw >> 48) & 0xFFFF) as u16,
        }
    }
}

pub trait ServerInternal {
    fn build_listener(&self, addr: &str) -> io::Result<TcpListener> {
        let listener = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
        // listener.set_tcp_nodelay(true)?;
        listener.set_reuse_address(true)?;
        listener.set_reuse_port(true)?;
        listener.bind(&SockAddr::from(
            addr.parse::<std::net::SocketAddr>().unwrap(),
        ))?;
        listener.listen(32768)?;
        listener.set_nonblocking(true)?;
        Ok(listener.into())
    }

    fn build_uring(&self, size: u32, sqpoll_idle: u32, affinity: u32, use_sqpoll: bool) -> io::Result<Uring> {
        Uring::new(size, sqpoll_idle, affinity, use_sqpoll)
    }
}