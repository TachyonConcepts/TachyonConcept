#![feature(vec_deque_truncate_front)]
#![feature(asm_experimental_arch)]
#![feature(array_ptr_get)]
#![allow(unsafe_op_in_unsafe_fn)]

use mimalloc::MiMalloc;
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use crate::library::server::{self, Server};
use tracing_subscriber::fmt;
use std::env::args;

pub mod library;

fn bootstrap_logs() {
    fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_target(false)
        .compact()
        .with_ansi(true)
        .init();
}

fn main() {
    bootstrap_logs();
    let server: Server = Server::new("0.0.0.0:8080")
        .set_sqpoll_enabled(false)
        .set_sqpoll_idle(0)
        .set_uring_size(4096)
        .set_realtime(false)
        // .set_workers(1) // num_cpus::get() as u8 / 2
        .set_ub_kernel_dma(args().any(|arg| arg == "--ubdma"))
        .build();

    server::run(server).unwrap();
}
