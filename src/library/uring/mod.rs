pub mod kernel_cmds;

use io_uring::{Builder, IoUring, cqueue, squeue}; // The magic portal to kernel-space IO wizardry.
use std::io;
use tracing::info;

pub struct Uring {
    pub uring: IoUring<squeue::Entry, cqueue::Entry>,
}

impl Uring {
    pub fn new(
        size: u32,            // Depth of the ring buffer. How much pain you can queue.
        sqpoll_idle: u32, // How long the SQ thread should wait (in milliseconds) before chilling out.
        affinity: u32,    // CPU core to pin the SQPOLL thread to. Because cache locality is king.
        sqpoll_enabled: bool, // Whether to unleash the SQPOLL daemon.
    ) -> io::Result<Uring> {
        let mut builder: Builder = IoUring::builder();
        // This makes sure only one thread submits SQEs at a time.
        // Think of it as "no cut in line" for submissions.
        builder.setup_single_issuer();
        if sqpoll_enabled {
            // Welcome to SQPOLL mode: where the kernel spawns a thread to babysit your submission queue.
            info!("SQPOLL enabled");
            info!("    Uring SQPOLL idle: {}", sqpoll_idle);
            info!("    Uring SQPOLL affinity: {}", affinity);
            // Tell kernel how long to keep polling before taking a nap.
            builder.setup_sqpoll(sqpoll_idle);
            // Pin the polling thread to a specific CPU. So it doesn't wander off into bad cache land.
            builder.setup_sqpoll_cpu(affinity);
            // Prevent the ring from being inherited across fork().
            // Because zombies and file descriptors don’t mix.
            builder.dontfork();
        } else {
            // Without SQPOLL, we want every submission to be flushed immediately.
            // No lazy batching — just send it!
            builder.setup_submit_all();
        }
        // Try to build the uring. If this fails, you probably forgot to sacrifice a goat to the kernel.
        let uring: IoUring<squeue::Entry, cqueue::Entry> = builder.build(size)?;
        Ok(Uring { uring })
    }
}
