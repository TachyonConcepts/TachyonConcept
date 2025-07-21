#![feature(vec_deque_truncate_front)]
#![feature(asm_experimental_arch)]
#![feature(array_ptr_get)]
#![allow(unsafe_op_in_unsafe_fn)]

use mimalloc::MiMalloc;
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

pub mod library;
use library::{
    server::{self, Server},
    utils::tachyon_data_lake::{TachyonDataLake, TachyonDataLakeTools},
};
use std::env::args;
use tachyon_json::{tachyon_object_noescape, TachyonBuffer, TachyonValue};
use tracing_subscriber::fmt;

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

const STATUS_SUCCESS: &[u8] = b"HTTP/1.1 200 OK\r\n";
const STATUS_NOT_FOUND: &[u8] = b"HTTP/1.1 404 Not Found\r\n";
const CONTENT_TYPE_TEXT: &[u8] = b"Content-Type: text/plain; charset=utf-8\r\n";
const CONTENT_TYPE_JSON: &[u8] = b"Content-Type: application/json; charset=utf-8\r\n";

const BASE_HEADERS: &[u8] = b"Server: Tachyon\r\n\
Connection: keep-alive\r\n\
Keep-Alive: timeout=5, max=1000\r\n\
\r\n";

impl Server {
    pub unsafe fn handler<'a>(
        method: &[u8],
        path: &[u8],
        date: &[u8; 35],
        hot_cache: &mut [u8],
        json_buf: &mut TachyonBuffer<100>,
        data_lake: &mut TachyonDataLake<230>,
    ) -> usize {
        json_buf.reset_pos();
        data_lake.reset_pos();

        let (status_line, ct, body): (&[u8], &[u8], &[u8]);

        if method == b"GET" && path == b"/plaintext" {
            (status_line, ct, body) = (STATUS_SUCCESS, CONTENT_TYPE_TEXT, b"Hello, World!");
        } else if method == b"GET" && path == b"/json" {
            let msg: TachyonValue = tachyon_object_noescape! {
                "message" => "Hello, World!",
            };
            msg.encode(json_buf, true);
            (status_line, ct, body) = (STATUS_SUCCESS, CONTENT_TYPE_JSON, json_buf.as_slice())
        } else {
            (status_line, ct, body) = (STATUS_NOT_FOUND, CONTENT_TYPE_TEXT, b"Not, found!")
        };

        data_lake.write(status_line.as_ptr(), status_line.len());
        data_lake.write(ct.as_ptr(), ct.len());
        data_lake.write(date.as_ptr(), date.len());
        data_lake.write(b"\r\n".as_ptr(), 2);
        data_lake.write(b"Content-Length: ".as_ptr(), 16);
        data_lake.write_num_str_fixed(body.len(), 2);
        data_lake.write(b"\r\n".as_ptr(), 2);
        data_lake.write(BASE_HEADERS.as_ptr(), BASE_HEADERS.len());
        data_lake.write(body.as_ptr(), body.len());
        TachyonDataLakeTools::write_to(hot_cache.as_mut_ptr(), data_lake.as_ptr(), data_lake.len());
        data_lake.len()
    }
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
