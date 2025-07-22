use crate::library::{
    network::socket_helpers::prepare_incoming_socket,
    server_internals::{
        ServerInternal, UserData, BUFFER_REGISTER_CODE, CODE_ACCEPT, INIT_REQUEST,
        POLL_EVENT, REQ_RESP_OFFSET,
    },
    uring::{
        kernel_cmds::{accept_multi, poll_add, provide_buffer, recv_multi, send},
        Uring,
    },
    utils::{
        faf_helpers::attach_reuseport_cbpf,
        http::unreliable_parse_http_methods_paths,
        http::{parse_http_methods_paths, RequestEntry},
        kernel::log_kernel_error,
        shift::shift_ub_inplace,
        tachyon_data_lake::TachyonDataLake,
        trim::l_trim256,
    },
};
use core_affinity::CoreId;
use io_uring::{cqueue, squeue::Entry, CompletionQueue, SubmissionQueue, Submitter};
use libc::{setsockopt, ENOBUFS, IPPROTO_IP, IP_TOS};
use nano_clock::{nano_http_date, nano_timestamp, timestamp};
use stable_vec::ExternStableVec;
use std::{
    collections::VecDeque,
    io,
    net::TcpListener,
    os::fd::{AsRawFd, RawFd},
    thread,
};
use std::time::Duration;
use tachyon_json::TachyonBuffer;
use thread_priority::{ThreadBuilderExt, *};
use tracing::{error, info, trace};

pub(crate) const BUFFER_SIZE: usize = 7168; // 6272 7168
const BUFFERS_COUNT: usize = 1024;
const DEFAULT_URING_SIZE: u32 = 4096;
const DEFAULT_ACCEPT_MULTIPLICATOR: u8 = 16;
const DEFAULT_SQPOLL_IDLE: u32 = 5000;
const DATA_LAKE_SIZE: usize = BUFFER_SIZE * 2; // 4100

#[derive(Copy, Clone)]
pub struct Ptr<T>(*const T);

impl<T> Ptr<T> {
    pub fn new(ptr: *const T) -> Self {
        Self(ptr)
    }
    pub fn get(&self) -> *const T {
        self.0
    }
    pub unsafe fn as_ref<'a>(&self) -> &'a T {
        &*self.0
    }
}

#[derive(Clone)]
pub struct Server {
    // Public config
    addr: &'static str,
    workers: u8,
    uring_size: u32,
    sqpoll_enabled: bool,
    sqpoll_idle: u32,
    realtime: bool,
    ub_kernel_dma: bool,
    // Internal
    client_fds: ExternStableVec<RawFd>,

    pub(crate) date: [u8; 35],
    hot_json_buf: TachyonBuffer<100>,
    hot_data_lake: TachyonDataLake<230>,

    sync_now: bool,
    pointers: VecDeque<Ptr<TachyonDataLake<DATA_LAKE_SIZE>>>,
    buffers: [[u8; BUFFER_SIZE]; BUFFERS_COUNT],
    released_buffers: Vec<u16>,
    client_out_buffers: ExternStableVec<TachyonDataLake<DATA_LAKE_SIZE>>,
    client_kernel_buffer_id: ExternStableVec<u16>,
    hot_internal_cache: [u8; BUFFER_SIZE],
    universal_counter: usize,
    io_send_busy: bool,
    // Internal clock
    nano_clock: i64,
    clock: i64,
    last_sync_time: i64,
    // RPS and Hz
    rps: u64,
    hz: u64,
    synced: i64,
    _rps: u64,
    _hz: u64,
    //Device
    _mac: String,
    _pci: String,
}
impl ServerInternal for Server {}
unsafe impl Send for Server {}
// Server engine / internal methods
impl Server {
    /// # Safety:
    /// This function dances with the kernel using raw pointers and unholy rituals.
    /// It assumes that `self.buffers` are properly allocated and aligned.
    /// Call it only if you know what you're doing — or have accepted your fate.
    unsafe fn register_buffers(
        &mut self,
        sq: &mut SubmissionQueue,
        submitter: &Submitter,
    ) -> io::Result<()> {
        trace!("Registering buffers");
        // Prepare the kernel offering plate: N buffer registration requests
        let rqs: [Entry; BUFFERS_COUNT] = std::array::from_fn(|i| {
            trace!("    Registering buffer {} ({} bytes)", i, BUFFER_SIZE);
            let ptr: *mut u8 = self.buffers[i].as_mut_ptr();
            // Give the kernel a chunk of our soul (and memory)
            provide_buffer(ptr, i as u16, BUFFER_SIZE as i32)
        });
        trace!(
            "Total reserved {} bytes for kernel",
            BUFFER_SIZE * BUFFERS_COUNT
        );
        trace!("Send to kernel...");
        // Retry loop in case kernel isn't feeling cooperative today
        loop {
            match sq.push_multiple(rqs.as_slice()) {
                Ok(_) => break,
                Err(err) => {
                    error!("Push buffers failed: {}", err);
                }
            }
        }
        // Sync SQE state to the kernel
        sq.sync();
        // Block until all buffers are acknowledged by the Great Ring
        submitter.submit_and_wait(BUFFERS_COUNT)?;
        trace!("Buffers successfully registered in kernel");
        Ok(())
    }

    /// # Safety:
    /// Returns previously kernel-owned buffers back to the altar,
    /// so they can once again serve as humble vessels for incoming packets.
    /// If you mess this up — the kernel *will* remember.
    unsafe fn release_buffers(
        &mut self,
        sq: &mut SubmissionQueue,
        submitter: &Submitter,
    ) -> io::Result<()> {
        if self.released_buffers.len() == 0 {
            return Ok(());
        }
        trace!("Return buffers");
        submitter.submit()?;
        let mut rqs: Vec<Entry> = Vec::with_capacity(self.released_buffers.len());
        for id in self.released_buffers.iter() {
            trace!("    Return buffer {} ({} bytes)", id, BUFFER_SIZE);
            let ptr: *mut u8 = self.buffers[*id as usize].as_mut_ptr();
            rqs.push(provide_buffer(ptr, *id, BUFFER_SIZE as i32));
        }
        trace!("Send to kernel...");
        // Spam the kernel until it finally takes our offering
        loop {
            match sq.push_multiple(rqs.as_slice()) {
                Ok(_) => break,
                Err(err) => {
                    error!("Push buffers failed: {}", err);
                }
            }
        }
        // Let the kernel know we’re done submitting. For now.
        sq.sync();
        // Block until the kernel acknowledges every last byte
        submitter.submit_and_wait(self.released_buffers.len())?;
        trace!("Buffers successfully registered in kernel");
        // The sacrifice is complete. Purge all traces.
        self.released_buffers.clear();
        Ok(())
    }

    /// Accept the chosen one. Or just another socket.
    ///
    /// This function is called when the kernel delivers us a shiny new client via `accept`.
    /// We take it lovingly, log it, set up the housewarming (recv and poll), and send it off
    /// into the cruel world of io_uring.
    ///
    /// UBDMA twist:
    /// If we’re in “dark magic mode” (ub_kernel_dma), we pre-arm a `POLL_ADD` right after accepting,
    /// because we’re not going to wait for boring `recv` completions like civilized people.
    /// We want to look directly into the kernel’s soul — or at least its socket buffer.
    ///
    /// Note: We also store the client’s output buffer (we’ll need it later to scream responses back).
    unsafe fn process_entry_accept(
        &mut self,
        sq: &mut SubmissionQueue,
        result: i32,
    ) -> io::Result<()> {
        trace!("Accept request");
        let client_fd: i32 = result;
        if client_fd <= 0 {
            // Client tried to connect, kernel said "nope".
            error!("Connection accept error on FD:{client_fd}");
            return Ok(());
        }
        // Prep the client socket — disable Nagle, make it raw and fast and angry.
        prepare_incoming_socket(client_fd);
        // Store the file descriptor and assign it a logical ID (our own personal client index).
        let client_fd_id = self.client_fds.push(client_fd);
        // Allocate a per-client outgoing buffer. We don’t write yet, but it’s good to be ready.
        self.client_out_buffers
            .insert(client_fd_id, TachyonDataLake::<DATA_LAKE_SIZE>::build());
        trace!("Receive new accept on FD:{client_fd}. Connection ID: {client_fd_id}");
        // If UBDMA is enabled — we go turbo mode.
        // Instead of waiting for recv to finish, we slap a poll here and later just read the buffer directly.
        if self.ub_kernel_dma {
            let client_poll_flag = UserData {
                client_id: client_fd_id as u32,
                buffer_id: 0,
                uniq_id: POLL_EVENT,
            };
            let poll: Entry = poll_add(client_fd, client_poll_flag.pack_user_data());
            sq.push(&poll).unwrap_or(()); // If this fails, we pretend nothing happened. YOLO.
            sq.sync(); // Let kernel know we’re serious about this poll.
        }
        // Schedule initial recv (multi-shot, of course — we’re not cavepeople).
        let user_data: u64 = UserData {
            client_id: client_fd_id as u32,
            buffer_id: 0,
            uniq_id: INIT_REQUEST,
        }
        .pack_user_data();
        sq.push(&recv_multi(client_fd, user_data)).unwrap_or(());
        // Force the uring loop to flush to the kernel now.
        self.sync_now = true;
        Ok(())
    }

    /// The sacred wideband_send() ritual
    ///
    /// This function handles outgoing data like a postal worker on five shots of espresso.
    /// It loops through every client with something to say and throws their packets at the kernel
    /// as fast as it dares — or until the RPS gods say “chill”.
    ///
    /// In UBDMA mode, all throttling logic is ignored because safety is for people who write Java.
    ///
    /// Notes:
    /// - We store frozen buffers in a `pointers` deque to avoid lifetime issues.
    /// - If that deque becomes too long, we truncate it like a corrupt politician’s résumé.
    /// - We try to avoid overloading the kernel unless we’re told otherwise (via UBDMA).
    unsafe fn wideband_send(&mut self, sq: &mut SubmissionQueue) -> io::Result<()> {
        // We're hoarding old responses like dragons hoard gold — time to burn some.
        if self.pointers.len() >= 100_000 {
            trace!("Truncate pointers");
            self.pointers.truncate_front(20_000);
        }

        if !self.ub_kernel_dma {
            let diff: i64 = self.nano_clock - self.last_sync_time;
            let mut stopper = 0;
            // Dynamic backpressure logic: throttle based on current RPS level.
            // The more chaos we cause, the more the server asks us to stop.
            if self.rps > 500_000 {
                stopper = 1_000;
            }
            if self.rps > 1_000_000 {
                stopper = 2_000;
            }
            if diff < stopper {
                self.universal_counter += 1;
                return Ok(());
            }
            self.last_sync_time = self.nano_clock;
        }

        trace!("Call wideband send");
        // Are we sending data to anyone, or just talking to ourselves again?
        let used_out_buffers_len = self.client_out_buffers.iter().len();
        if used_out_buffers_len == 0 {
            trace!("Nothing to send");
            return Ok(());
        }
        self.last_sync_time = self.nano_clock;
        // Collect all outgoing payloads and turn them into send entries.
        let mut entries: Vec<Entry> = Vec::with_capacity(100);
        for (client_id, data_lake) in self.client_out_buffers.iter_mut() {
            if data_lake.len() == 0 {
                continue;
            }
            let lake_ptr: *const TachyonDataLake<DATA_LAKE_SIZE> = data_lake.freeze_ptr();
            // Do we still know this client? Or did it rage-quit the universe?
            match self.client_fds.get(client_id) {
                Some(fd) => {
                    let fd: RawFd = *fd;
                    // Store the frozen packet to keep it alive during send.
                    self.pointers.push_back(Ptr::new(lake_ptr));
                    let new_ptr: &Ptr<TachyonDataLake<DATA_LAKE_SIZE>> =
                        self.pointers.back().unwrap();
                    // Build the sacred send entry.
                    let send_entry: Entry = send(
                        UserData {
                            client_id: client_id as u32,
                            buffer_id: 0,
                            uniq_id: 0xCCCu16,
                        }
                        .pack_user_data(),
                        fd,
                        new_ptr.as_ref().as_slice(),
                        false,
                    );
                    entries.push(send_entry);
                }
                None => {
                    error!("Buffer {} not found!", client_id);
                    continue;
                }
            };
            data_lake.reset_pos();
        }
        if entries.len() > 0 {
            sq.push_multiple(&entries).unwrap();
            self.io_send_busy = true;
        }
        // Mark the system as dirty — we’ll need to flush again soon.
        self.sync_now = true;
        Ok(())
    }

    /// Welcome to UBDMA mode: Undefined Behavior Direct Memory Access.
    ///
    /// This is not your grandmother’s networking server. This is an
    /// unapologetic, high-throughput berserker mode where the only
    /// concern is *how much* can be processed, *how fast*, and to hell
    /// with the rules.
    ///
    /// In the civilized world, `io_uring` wants us to wait politely for a `recv`
    /// to complete. We call `provide_buffers`, wait for a POLL event,
    /// then issue a proper `recv` and parse the resulting buffer like law-abiding citizens.
    ///
    /// But not here.
    ///
    /// In UBDMA mode, we don't wait. As soon as POLL says “ready,”
    /// we brute-force our way straight into the last known buffer
    /// *before the kernel even finishes writing to it*. We copy whatever
    /// half-written black magic we find, perform a quick `shift()` to
    /// make room for new garbage, and if what we grabbed looks vaguely
    /// like an HTTP request — *we run with it*.
    ///
    /// This is raw, degenerate optimism. This is reading memory
    /// during a DMA write and pretending that’s fine.
    ///
    /// If it crashes? Who cares.
    /// If the request is broken? We throw it away.
    /// If we accidentally parse a cat as a GET request? Welcome to UBDMA.
    ///
    /// This mode is useful only when you're chasing **absurd RPS numbers**,
    /// where a few broken requests are acceptable sacrifices to the throughput gods.
    ///
    /// If you're still reading this and thinking “that’s a bad idea” — yes, it is.
    /// That’s the point.
    ///
    ///        ☠ UBDMA ☠
    ///     ———————————————
    ///     |    DANGER!     |
    ///     | Entering land |
    ///     |  of segfaults  |
    ///     |   and demons   |
    ///     ———————————————
    ///           (╯°□°）╯︵ ┻━┻
    /// In conclusion: this works *only* because io_uring gives you dangerous freedom.
    /// It's not documented behavior. It’s just behavior. You saw the light turn green — so you stepped on the gas while the bus was still turning.
    unsafe fn ubdma(&mut self, client: UserData) -> io::Result<()> {
        trace!("Enter EB-DMA");
        // We begin our descent into madness. First, identify the chosen victim.
        let cid: usize = client.client_id as usize;
        // Retrieve the sacred kernel buffer ID. If it doesn’t exist, the ritual cannot proceed.
        let kernel_buffer: Option<&u16> = self.client_kernel_buffer_id.get(cid);
        if kernel_buffer.is_none() {
            trace!("No kernel buffer found for client {}", cid);
            return Ok(());
        }
        // Lock in on the buffer. This is the forbidden fruit we shall bite *while the kernel is still peeling it*.
        let kernel_buffer_id: usize = *kernel_buffer.unwrap() as usize;
        let buffer: &[u8] = &self.buffers[kernel_buffer_id][..];
        // Try to make sense of the data inside while it’s still twitching. Extract HTTP requests.
        let buffer: &[u8] = l_trim256(buffer);
        let requests: ([RequestEntry; 50], usize) = unreliable_parse_http_methods_paths(&buffer);
        // Hypothetically useful FDs (e.g. if we needed to set priority for latecomers).
        let useful: Vec<RawFd> = Vec::with_capacity(requests.1.saturating_sub(5) * 128);
        // If we’re not busy, boost the client's priority — might be real traffic!
        if useful.capacity() == 0 {
            trace!("Incr priority for {}", client.client_id);
            let tos: libc::c_int = 0xB8;
            let cfid: Option<&RawFd> = self.client_fds.get(cid);
            if cfid.is_some() {
                setsockopt(
                    cfid.unwrap().as_raw_fd(),
                    IPPROTO_IP,
                    IP_TOS,
                    &tos as *const _ as *const _,
                    size_of_val(&tos) as _,
                );
            }
            return Ok(());
        }
        // Just in case the client ran off before we could send the punchline.
        let cbstate: Option<&mut TachyonDataLake<DATA_LAKE_SIZE>> =
            self.client_out_buffers.get_mut(cid);
        if cbstate.is_none() {
            error!("Client buffer not found. Client disconnected?");
            return Ok(());
        }
        // If there's already data pending — someone else beat us to the chaos.
        let client_buffer: &mut TachyonDataLake<DATA_LAKE_SIZE> = cbstate.unwrap();
        if client_buffer.len() != 0 {
            trace!("Some request already processed. Skip.");
            return Ok(());
        }
        // We're in! Kernel has placed fresh bytes in the buffer. Now we taste the forbidden entropy.
        trace!("Catch! Kernel post new data");
        let mut total_len: usize = 0;
        for index in 0..requests.1 {
            self._rps += 1;
            let request: &RequestEntry = requests.0.get(index).unwrap();
            let len: usize = Self::handler(
                request.0,
                request.1,
                &self.date,
                &mut self.hot_internal_cache[total_len..],
                &mut self.hot_json_buf,
                &mut self.hot_data_lake,
            );
            // println!("{}", String::from_utf8_lossy(&self.hot_internal_cache));
            total_len += len;
        }
        // And now... PUSH! Like the buffer owes us money.
        let hot_slice = &self.hot_internal_cache[..total_len];
        client_buffer.write(hot_slice.as_ptr(), hot_slice.len());
        // We notify the outer loop that things have happened. Dark things.

        self.sync_now = true;
        Ok(())
    }
    unsafe fn request_reply(
        &mut self,
        flags: u32,
        user_data: UserData,
        sq: &mut SubmissionQueue,
        result: i32,
        submitter: &Submitter,
    ) -> io::Result<()> {
        // Extract buffer ID from upper 16 bits of flags. Not suspicious at all.
        let buf_id: u16 = (flags >> 16) as u16;
        let cid: usize = user_data.client_id as usize;
        // If UBDMA mode is enabled, remember where the kernel is about to... do things.
        if self.ub_kernel_dma {
            self.client_kernel_buffer_id.insert(cid, buf_id);
        }
        // Sanity check: if result > buffer size, something has gone *very* wrong. Likely aliens.
        if result > BUFFER_SIZE as i32 {
            error!("Incorrect packet length > {}", BUFFER_SIZE);
            return Ok(());
        }
        // Safely borrow the unholy slab of bytes the kernel just dumped on us.
        let buffer: &[u8] = &self.buffers[buf_id as usize][..result as usize];
        trace!("New message incoming. Len: {}", buffer.len());
        // Attempt to extract structured requests from the raw chaos.
        let requests: ([RequestEntry; 50], usize) = parse_http_methods_paths(buffer);
        // Begin preparing a response. Fast-path for success and not-so-fast for "not found".
        let mut total_len: usize = 0;
        for index in 0..requests.1 {
            self._rps += 1;
            let request: &RequestEntry = requests.0.get(index).unwrap();
            let len: usize = Self::handler(
                request.0,
                request.1,
                &self.date,
                &mut self.hot_internal_cache[total_len..],
                &mut self.hot_json_buf,
                &mut self.hot_data_lake,
            );
            // println!("{}", String::from_utf8_lossy(&self.hot_internal_cache));
            total_len += len;
        }
        // Fire the prepared response payload into the client's output buffer.
        let hot_slice: &[u8] = &self.hot_internal_cache[..total_len];
        let client_buffer: &mut TachyonDataLake<DATA_LAKE_SIZE> =
            self.client_out_buffers.get_unchecked_mut(cid);
        client_buffer.write(hot_slice.as_ptr(), hot_slice.len());
        // Flag for sync: this shall be pushed soon.
        self.sync_now = true;
        // Return buffer to "available" list. Just not yet — batching is everything.
        self.released_buffers.push(buf_id);
        // And now, the cursed part:
        if self.ub_kernel_dma {
            // Check the byte at the tail of the buffer. If it’s 0, assume "safe to shift".
            let buf: &mut [u8; BUFFER_SIZE] = &mut self.buffers[buf_id as usize];
            if buf[BUFFER_SIZE - 1] == 0 {
                shift_ub_inplace(buf, result as usize);
            }
        }
        // If we’ve hoarded enough buffers, release the Kraken.
        let part: f64 = BUFFERS_COUNT as f64 / 1.5;
        if self.released_buffers.len() == part as usize {
            self.release_buffers(sq, submitter)?;
        }
        Ok(())
    }
    unsafe fn close_connection(&mut self, client_id: usize, force: bool) -> io::Result<()> {
        // Let the logs know this poor soul is being disconnected.
        trace!("FD closed for client ID: {client_id}");
        // Default to a cursed value. If we use this, something has gone terribly wrong.
        let mut cfd: RawFd = 0xFF;
        if !force {
            // If not forced, we’re just politely checking whether the client still exists.
            let fd: Option<RawFd> = self.client_fds.get(client_id).cloned();
            if let Some(fd) = fd {
                cfd = fd;
            }
        } else {
            // Emergency disconnect. No questions asked.
            trace!("Closing connection with ID {} FORCED", client_id);
            // Yank the file descriptor from the pool of the living.
            if let Some(fd) = self.client_fds.remove(client_id) {
                cfd = fd;
            }
            // Also delete their precious outbound buffer. We’re done being nice.
            self.client_out_buffers.remove(client_id);
        }
        // If someone tries to close STDIN — we say no. Even if we're wild, we're not *that* wild.
        if cfd == 0 {
            error!("Attempt to close STDIN FD!!! Permission Denied!");
            return Ok(());
        }
        // Perform the sacred syscall ritual. RIP, file descriptor.
        libc::close(cfd);
        Ok(())
    }
    unsafe fn process_entry_response(
        &mut self,
        user_data: u64,
        result: i32,
        flags: u32,
        sq: &mut SubmissionQueue,
        submitter: &Submitter,
    ) -> io::Result<()> {
        // Extract the mysterious metadata packed in u64 (client ID, buffer ID, uniq op)
        let user: UserData = UserData::unpack_user_data(user_data);
        let client_id: usize = user.client_id as usize;

        // If the kernel gives us nothing or says "bad fd" — pretend we didn’t see anything.
        if result == -libc::EAGAIN || result == -libc::EBADF {
            return Ok(());
        }

        // Classic out-of-buffers panic. Don’t scream — just free some memory and pray.
        if result == -ENOBUFS {
            error!("Buffers end! Server feel bad :((((");
            self.release_buffers(sq, submitter)?;
            return Ok(());
        }

        // When the kernel says -32 for reasons unknown to mankind (usually timeout), we shrug.
        if result == -32 {
            return Ok(());
        }

        // Gracefully close connection if result is 0 (client closed) and it’s not a send op.
        // Or if the kernel hits us with a connection reset slap.
        if result == 0 && user.uniq_id != 0xCCC || result == -libc::ECONNRESET {
            trace!("Close CONN {} on client {} RESET", result, client_id);
            self.close_connection(client_id, true)?;
            return Ok(());
        }

        // Any other error is logged and ignored like a good soldier ignoring chaos.
        if result < 0 {
            error!("Received negative result from kernel: {result}");
            return Ok(());
        }

        // If this is a poll event and we are in UBDMA mode... we go off-script.
        if user.uniq_id == POLL_EVENT && self.ub_kernel_dma {
            self.ubdma(user)?;
            return Ok(());
        }

        // If the result came from a send operation, do nothing. We don’t celebrate sending.
        if user.uniq_id == 0xCCC {
            return Ok(());
        }

        // Otherwise, process the actual request and build a majestic reply.
        if result > 0 {
            self.request_reply(flags, user, sq, result, submitter)?;
        }
        Ok(())
    }
    unsafe fn process_entry(
        &mut self,
        cqe: cqueue::Entry,
        mut sq: &mut SubmissionQueue,
        submitter: &Submitter,
    ) -> io::Result<()> {
        // Unpack the sacred scrolls from the Completion Queue Entry.
        let user_data: u64 = cqe.user_data();
        let result: i32 = cqe.result();
        let flags: u32 = cqe.flags();
        // A very boring but necessary special case: kernel acknowledged our buffer registration.
        // Yay. Move along.
        if user_data == BUFFER_REGISTER_CODE && result == 0 && flags == 0 {
            trace!("Buffer registered in kernel");
            return Ok(());
        }
        // Now for the exciting part: routing this mysterious event to its handler.
        match user_data {
            // The birth of a connection: someone dared to connect to us.
            CODE_ACCEPT => self.process_entry_accept(&mut sq, result)?,
            // The main pipeline: recv, poll, ubdma, send – everything that happens after connect.
            user_data if user_data >= REQ_RESP_OFFSET => {
                self.process_entry_response(user_data, result, flags, sq, submitter)?
            }
            // If you ended up here, either you’re debugging, hallucinating, or the kernel is.
            _ => {
                trace!("Unmapped user data: {user_data}");
            }
        };
        Ok(())
    }
    fn reserve_writes_buffer(&mut self, reserve: usize) {
        trace!("Reserve write buffer");
        for _ in 0..reserve {
            self.client_fds.push(0xFFFF);
        }
    }
    unsafe fn sq_poll(
        &mut self,
        listener: TcpListener,
        sqpoll_idle: u32,
        affinity: u32,
    ) -> io::Result<()> {
        // Welcome to the beating heart of the reactor — aka the Kernel Summoning Loop™.
        // Here we prep the altar, draw the circles (Uring, SQ, CQ), and start the sacred dance of syscalls.
        info!("Build Uring instance.");
        let mut uring: Uring =
            self.build_uring(self.uring_size, sqpoll_idle, affinity, self.sqpoll_enabled)?;
        info!("Success. Split Uring.");
        let (submitter, mut sq, mut cq): (Submitter, SubmissionQueue, CompletionQueue) =
            uring.uring.split();
        // Give the kernel its offering: a set of sacrificial buffers.
        self.register_buffers(&mut sq, &submitter)?;
        // Prepare the holy socket
        let listener_fd: RawFd = listener.as_raw_fd();
        attach_reuseport_cbpf(listener_fd as isize);
        trace!("Listener fd: {listener_fd}");
        info!("Start multi accept");
        for i in 0..DEFAULT_ACCEPT_MULTIPLICATOR {
            info!("    Start multi accept #{i}");
            sq.push(&accept_multi(listener_fd)).unwrap(); // bless this socket with many accepts
        }
        info!("Submit all changes to kernel.");
        submitter.submit()?; // hand everything over to the dark overlord
        info!("Kernel ready");
        // Pre-fill your outbound rocket launcher.
        self.reserve_writes_buffer(100);
        // Register dummy FDs — the kernel demands it. We comply. We always comply.
        let fds: [RawFd; 1024] = [-1; 1024];
        submitter.register_files(&fds)?;
        self.synced = 0;
        loop {
            self.nano_clock = nano_timestamp();
            self.clock = timestamp();
            // Per-second metrics update
            if self.clock != self.synced {
                self.rps = self._rps;
                self.hz = self._hz;
                self._rps = 0;
                self._hz = 0;
                self.synced = self.clock;
                // Format timestamp, broadcast our existence
                let mut date: [u8; 35] = [0; 35];
                nano_http_date(&mut date, false);
                self.date = date;
                let conns = self.client_fds.iter().len() - 100; // forgive the arbitrary 100, it knows what it did
                info!(
                    "Server: RPS: {} kHz: {} CQ: {} SQ: {} CONNS: {} UC: {}",
                    self.rps,
                    self.hz / 1000,
                    cq.len(),
                    sq.len(),
                    conns,
                    self.universal_counter
                );
            }
            // If no CQE yet, try to flush outbound sends
            if cq.is_empty() {
                self.wideband_send(&mut sq)?;
            }
            trace!("New runtime iteration");
            // Sync SQ if we queued something in this loop
            if self.sync_now {
                trace!("SQ synchronization");
                sq.sync();
                self.sync_now = false;
            } else {
                trace!("Skip SQ synchronization");
            }
            // Sync CQ if it's still empty (just in case the kernel is feeling shy)
            if cq.is_empty() {
                cq.sync();
            }
            // Decide how we wake the kernel — gently or with a slap
            if self.io_send_busy || (!self.io_send_busy && cq.is_empty()) {
                // Wait for at least one CQE – because we're lonely
                submitter.submit_and_wait(1)?;
                self.io_send_busy = false;
            } else {
                // Fire and forget
                submitter.submit()?;
            }
            self._hz += 1;
            // Process each gift the kernel brings us
            while let Some(cqe) = cq.next() {
                trace!("New CQE: {:?}", cqe);
                self.process_entry(cqe, &mut sq, &submitter)?;
                self._hz += 1;
            }
            // If we’re bored (0 RPS), clean up the kitchen
            if self.rps == 0 {
                self.release_buffers(&mut sq, &submitter)?;
            }
        }
    }
}
// Public server endpoints
impl Server {
    pub fn new(addr: &'static str) -> Server {
        Server {
            addr,
            workers: num_cpus::get().max(1) as u8,
            uring_size: DEFAULT_URING_SIZE,
            sqpoll_idle: DEFAULT_SQPOLL_IDLE,
            sqpoll_enabled: false,
            realtime: false,
            ub_kernel_dma: false,
            client_fds: ExternStableVec::new(),
            date: [0u8; 35],
            hot_json_buf: TachyonBuffer::<100>::default(),
            hot_data_lake: TachyonDataLake::<230>::build(),
            sync_now: true,
            pointers: VecDeque::new(),
            buffers: [[0u8; BUFFER_SIZE]; BUFFERS_COUNT],
            released_buffers: Vec::with_capacity(BUFFERS_COUNT),
            client_out_buffers: ExternStableVec::with_capacity(u16::MAX as usize),
            client_kernel_buffer_id: ExternStableVec::with_capacity(u16::MAX as usize),
            hot_internal_cache: [0u8; BUFFER_SIZE],
            io_send_busy: false,
            universal_counter: 0,
            nano_clock: unsafe { nano_timestamp() },
            clock: unsafe { timestamp() },
            last_sync_time: 0,
            rps: 0,
            hz: 0,
            synced: 0,
            _rps: 0,
            _hz: 0,
            _mac: String::default(),
            _pci: String::default(),
        }
    }
    #[inline(always)]
    pub fn get_sqpoll_idle(&self) -> u32 {
        self.sqpoll_idle
    }
    #[inline(always)]
    pub fn get_workers(&self) -> u8 {
        self.workers
    }
    #[inline(always)]
    pub fn set_workers(&mut self, workers: u8) -> &mut Self {
        self.workers = workers;
        self
    }
    #[inline(always)]
    pub fn get_sqpoll_enabled(&self) -> bool {
        self.sqpoll_enabled
    }
    #[inline(always)]
    pub fn set_sqpoll_enabled(&mut self, enabled: bool) -> &mut Self {
        self.sqpoll_enabled = enabled;
        self
    }
    #[inline(always)]
    pub fn set_uring_size(&mut self, uring_size: u32) -> &mut Self {
        self.uring_size = uring_size;
        self
    }
    #[inline(always)]
    pub fn set_sqpoll_idle(&mut self, sqpoll_idle: u32) -> &mut Self {
        self.sqpoll_idle = sqpoll_idle;
        self
    }
    #[inline(always)]
    pub fn get_realtime(&self) -> bool {
        self.realtime
    }
    #[inline(always)]
    pub fn set_realtime(&mut self, enabled: bool) -> &mut Self {
        self.realtime = enabled;
        self
    }
    #[inline(always)]
    pub fn get_ub_kernel_dma(&self) -> bool {
        self.ub_kernel_dma
    }
    #[inline(always)]
    pub fn set_ub_kernel_dma(&mut self, enabled: bool) -> &mut Self {
        self.ub_kernel_dma = enabled;
        self
    }
    #[inline(always)]
    pub fn build(&mut self) -> Self {
        self.clone()
    }
}
pub fn run(server: Server) -> io::Result<()> {
    // Check for mutually exclusive flags — UBDMA cannot run in RT-safe environments
    if server.realtime && server.ub_kernel_dma {
        unsafe { log_kernel_error("realtime and ub dma is incompatible.", "RT_DMA_CONFLICT") };
        return Ok(());
    }
    // Yell at the user if UBDMA is enabled (because that's what you do before summoning chaos)
    if server.ub_kernel_dma {
        error!("*******************************");
        error!("* Server work in UNSAFE mode! *");
        error!("* !! This mode based on UB !! *");
        error!("*******************************");
        thread::sleep(Duration::from_secs(5)); // Give the user time to regret
    }
    // Spawn workers, bind to dedicated cores with max thread priority
    for thread in 0..server.get_workers() {
        let core_ids: Vec<CoreId> = core_affinity::get_core_ids().unwrap();
        info!("Thread {} starting", thread);
        let server = server.clone(); // yes, cloning entire server per-thread
        thread::Builder::new()
            .name(format!("Tachyon-{}", thread)) // Tachyon — fast, radioactive, and very real
            .stack_size(BUFFER_SIZE * BUFFERS_COUNT * 10) // big boy stack for big boy packets
            .spawn_with_priority(ThreadPriority::Max, move |_| unsafe {
                let res = core_affinity::set_for_current(core_ids[thread as usize]);
                if !res {
                    error!("Failed to set core affinity");
                } else {
                    info!(
                        "Core {} set affinity to {:?}",
                        thread, core_ids[thread as usize]
                    );
                }
                // The thread lives forever, unless panic takes it to Valhalla
                loop {
                    info!("Creating server instance");
                    let mut instance: Server = server.clone();
                    info!("Creating base listener");
                    let listener: TcpListener = instance.build_listener(instance.addr).unwrap();
                    if let Err(e) = instance.sq_poll(listener, server.sqpoll_idle, thread as u32) {
                        error!("worker error: {e}");
                    }
                }
            })?;
    }
    // Main thread parks itself like a good coordinator
    loop {
        thread::park();
    }
}
