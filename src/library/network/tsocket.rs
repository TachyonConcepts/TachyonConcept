use libc::{tpacket_versions::TPACKET_V3, *};
use std::{
    ffi::{CStr, CString},
    io,
    mem::size_of,
    net::Ipv4Addr,
    os::unix::io::RawFd,
    ptr, slice,
};
use std::borrow::Cow;

const FRAME_SIZE: usize = 8192; // One frame to rule them all... and hold some packets.
const FRAME_COUNT: usize = 4096; // Because why stop at a few? Let's go full chonky.
const BLOCK_SIZE: usize = FRAME_SIZE * FRAME_COUNT; // Total buffer size. Hope you brought RAM.

const BPF_TCP_FILTER: [sock_filter; 4] = [
    sock_filter {
        // Load the 23rd byte (a.k.a. protocol field in IPv4). Feels oddly specific? It is.
        code: (BPF_LD + BPF_B + BPF_ABS) as __u16,
        jt: 0,
        jf: 0,
        k: 23,
    },
    sock_filter {
        // Jump if it’s TCP (0x06) — because we only party with TCP.
        code: (BPF_JMP + BPF_JEQ + BPF_K) as __u16,
        jt: 0,
        jf: 1,
        k: 0x06,
    },
    sock_filter {
        // Accept the packet. Whole thing. Like a generous buffet plate.
        code: (BPF_RET + BPF_K) as __u16,
        jt: 0,
        jf: 0,
        k: u32::MAX,
    },
    sock_filter {
        // Reject the packet. Harshly. With zero mercy (and zero bytes).
        code: (BPF_RET + BPF_K) as __u16,
        jt: 0,
        jf: 0,
        k: 0,
    },
];
// Wrap it all in a sock_fprog — the ceremonial robe for our tiny BPF priesthood.
const BPF_TCP_PROG: sock_fprog = sock_fprog {
    len: BPF_TCP_FILTER.len() as u16,
    filter: BPF_TCP_FILTER.as_ptr() as *mut _,
};

#[derive(Clone)]
pub struct PacketRing {
    pub fd: RawFd, // File descriptor to the packet socket — our hotline to the kernel's buffet.
    pub mmap_ptr: *mut u8, // Pointer to the big scary shared memory blob.
    pub mmap_len: usize, // Size of said blob. Please don’t read past it. Seriously.
}

impl PacketRing {
    #[inline(always)]
    pub fn as_slice(&self) -> &[u8] {
        // Turn the raw pointer into a nice safe slice (just kidding, it's still unsafe).
        // If mmap_ptr is garbage, so is your future.
        unsafe { slice::from_raw_parts(self.mmap_ptr, self.mmap_len) }
    }
}

pub fn setup_packet_socket(iface: &str) -> io::Result<PacketRing> {
    // Create a raw packet socket. This is like saying “give me *everything* on this interface.”
    let fd = unsafe { socket(AF_PACKET, SOCK_RAW, htons(ETH_P_ALL as u16) as c_int) };
    if fd < 0 {
        return Err(io::Error::last_os_error()); // Kernel said nope.
    }
    // Tell the kernel we want TPACKET_V3. Because we like fast, weird, and memory-mapped.
    let version: c_uint = TPACKET_V3 as c_uint;
    let ret = unsafe {
        setsockopt(
            fd,
            SOL_PACKET,
            PACKET_VERSION,
            &version as *const _ as *const _,
            size_of::<c_uint>() as u32,
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    // Configure the ring buffer. This is where packets will be dumped, like log files in /tmp.
    let tp_req3 = tpacket_req3 {
        tp_block_size: BLOCK_SIZE as u32,
        tp_block_nr: 1,
        tp_frame_size: FRAME_SIZE as u32,
        tp_frame_nr: FRAME_COUNT as u32,
        tp_retire_blk_tov: 60, // ms
        tp_sizeof_priv: 0,
        tp_feature_req_word: 0,
    };
    let ret = unsafe {
        setsockopt(
            fd,
            SOL_PACKET,
            PACKET_RX_RING,
            &tp_req3 as *const _ as *const _,
            size_of::<tpacket_req3>() as u32,
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    // Convert interface name to index. You know, so the kernel knows which wire to sniff.
    let iface_cstr = CString::new(iface).unwrap();
    let if_index = unsafe { if_nametoindex(iface_cstr.as_ptr()) };
    if if_index == 0 {
        return Err(io::Error::last_os_error());
    }
    // Prepare the sockaddr_ll struct. It’s like RSVP-ing to a packet party.
    let sll: sockaddr_ll = sockaddr_ll {
        sll_family: AF_PACKET as u16,
        sll_protocol: htons(ETH_P_ALL as u16),
        sll_ifindex: if_index as i32,
        sll_hatype: 0,
        sll_pkttype: 0,
        sll_halen: 0,
        sll_addr: [0; 8],
    };
    // Bind the socket to the interface. You shall not sniff until you do this.
    let ret: c_int = unsafe {
        bind(
            fd,
            &sll as *const _ as *const sockaddr,
            size_of::<sockaddr_ll>() as u32,
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    // Attach our mighty BPF filter that lets only TCP through.
    let ret: c_int = unsafe {
        setsockopt(
            fd,
            SOL_SOCKET,
            SO_ATTACH_FILTER,
            &BPF_TCP_PROG as *const _ as *const _,
            size_of_val(&BPF_TCP_PROG) as u32,
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    // Map the ring buffer into our address space. Welcome to shared memory hell.
    let mmap_len: usize = BLOCK_SIZE;
    let mmap_ptr: *mut c_void = unsafe {
        mmap(
            ptr::null_mut(),
            mmap_len,
            PROT_READ | PROT_WRITE,
            MAP_SHARED,
            fd,
            0,
        )
    };

    if mmap_ptr == MAP_FAILED {
        return Err(io::Error::last_os_error()); // mmap didn't work. Time to cry.
    }

    // Success! You now have a direct line into kernel packet madness.
    Ok(PacketRing {
        fd,
        mmap_ptr: mmap_ptr as *mut u8,
        mmap_len,
    })
}

// Converts a 16-bit integer from host to network byte order.
// Because networks still haven't figured out endianness unity.
fn htons(value: u16) -> u16 {
    value.to_be()
}

// Flags for block ownership status — like a "Do Not Disturb" sign for memory.
const TP_STATUS_USER: u32 = 1; // Block belongs to user, hands off kernel!
const TP_STATUS_KERNEL: u32 = 0; // Kernel owns the block, user go away.

// Represents a block descriptor in TPACKET_V3.
// Basically, a header for a big shared memory slab full of packet goodies.
#[repr(C)]
#[derive(Debug)]
struct TpacketBlockDesc {
    version: u32, // Version of the tpacket — we're using V3, the final boss.
    offset_to_priv: u32, // Reserved space, in case you want to hide secrets in packets.
    hdr: TpacketHdrV1, // Main block metadata — who owns it, how many packets, etc.
}

// The actual block header (v1).
// Imagine a table of contents for everything in the block.
#[repr(C)]
#[derive(Debug)]
struct TpacketHdrV1 {
    block_status: u32, // Kernel or user? Who's currently hogging the block.
    num_pkts: u32, // How many packets are in this block? Hopefully not zero.
    offset_to_first_pkt: u32, // Where the first packet begins, because alignment is hard.
    blk_len: u32, // Total length of the block (including wasted dreams).
    seq_num: u64,  // Sequence number of the block. Not useful unless you're fancy.
    ts_first_pkt: u64, // Timestamp of the first packet — time travel begins here.
    ts_last_pkt: u64, // Timestamp of the last packet — time travel ends here.
}

// The per-packet header inside a block.
// Like a tiny envelope attached to every letter saying "hey, I'm packet-shaped!"
#[repr(C)]
#[derive(Debug)]
struct Tpacket3Hdr {
    tp_next_offset: u32, // Offset to the next packet (in case you still have energy).
    tp_sec: u32, // Packet timestamp: seconds part (for humans).
    tp_nsec: u32, // Packet timestamp: nanoseconds part (for robots).
    tp_snaplen: u32, // Captured length (might be smaller than full packet).
    tp_len: u32, // Actual length on the wire. This is the "real me".
    tp_status: u32, // Status flags — ownership, error, etc. Read with fear.
    tp_mac: u16, // Offset to the start of the Ethernet frame.
    tp_net: u16,  // Offset to the start of the IP header (if you're lucky).
}

pub fn get_ipv4_addr(interface: &str) -> Option<Ipv4Addr> {
    unsafe {
        let mut ifap: *mut ifaddrs = ptr::null_mut();
        // Ask the kernel for all the interfaces it knows about. It's like a gossip request.
        if getifaddrs(&mut ifap) != 0 {
            return None; // Something went wrong. Probably cursed network stack.
        }
        let mut cur: *mut ifaddrs = ifap;
        // Walk through the linked list of interfaces like it's 1993 and we're writing C.
        while !cur.is_null() {
            // Convert interface name to Rust string, because we hate raw pointers.
            let name: Cow<str> = CStr::from_ptr((*cur).ifa_name as *const c_char).to_string_lossy();
            // Check if this is the interface we’re looking for and if it has an IPv4 address.
            if name == interface
                && (*cur).ifa_addr != ptr::null_mut()
                && (*(*cur).ifa_addr).sa_family as i32 == AF_INET
            {
                // Hooray! Found an IPv4 address. Let's unwrap it like a present.
                let sin: &sockaddr_in = &*((*cur).ifa_addr as *const sockaddr_in);
                // Convert the raw `in_addr` to a nice friendly `Ipv4Addr`.
                let ip: Ipv4Addr = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                freeifaddrs(ifap); // Clean up like a good citizen.
                return Some(ip);
            }
            // Move to the next interface in the linked list of pain.
            cur = (*cur).ifa_next;
        }
        // We didn’t find anything, but at least we freed the memory.
        freeifaddrs(ifap);
        None
    }
}

pub unsafe fn read_packets(ring: &PacketRing) -> Option<(usize, usize)> {
    // Cast the start of the mmap'd region to a TpacketBlockDesc.
    let block: *mut TpacketBlockDesc = ring.mmap_ptr as *mut TpacketBlockDesc;
    // Is the block ready for us? Or is the kernel still hoarding it?
    let block_status: u32 = (*block).hdr.block_status;
    if block_status & TP_STATUS_USER == 0 {
        return None;
    }
    let num_pkts: u32 = (*block).hdr.num_pkts; // How many packets did the kernel gift us?
    let offset: u32 = (*block).hdr.offset_to_first_pkt; // Where does the first packet live?
    let mut base: *mut u8 = (ring.mmap_ptr).add(offset as usize);

    let ring_base: usize = ring.mmap_ptr as usize;
    let mut first_offset = None;
    let mut last_offset: usize = 0usize;
    // We need to know our own IP to avoid self-love (a.k.a. skipping loopback packets).
    let server_ip: Ipv4Addr = get_ipv4_addr("eth0").expect("Failed to get server IP");
    // Iterate over all the packets in this block.
    for _ in 0..num_pkts {
        let hdr: &Tpacket3Hdr = &*(base as *const Tpacket3Hdr);
        // Find where the actual Ethernet frame starts.
        let data_ptr: *mut u8 = base.add(hdr.tp_mac as usize);
        let data_len: usize = hdr.tp_snaplen as usize;
        let data_offset: usize = data_ptr as usize - ring_base;
        // Not even a full Ethernet header? Throw it out like a bad Tinder match.
        if data_len < 14 {
            continue;
        }
        // Check ethertype: 0x0800 means IPv4. Anything else? Meh, skip.
        let ethertype: u16 = u16::from_be_bytes([*data_ptr.add(12), *data_ptr.add(13)]);
        if ethertype != 0x0800 {
            continue;
        }
        // Not enough for an IP header? Nah.
        if data_len < 14 + 20 {
            continue;
        }
        // Extract the source IP from the IPv4 header. (It’s in a totally normal place. Trust us.)
        let ip_start: *mut u8 = data_ptr.add(14);
        let src_ip: Ipv4Addr = Ipv4Addr::new(
            *ip_start.add(12),
            *ip_start.add(13),
            *ip_start.add(14),
            *ip_start.add(15),
        );
        // Ignore packets from ourselves. Narcissism isn't packet-safe.
        if src_ip == server_ip {
            continue;
        }
        // Mark the first packet offset, if we haven’t already.
        if first_offset.is_none() {
            first_offset = Some(data_offset);
        }
        // Update the last offset to include the current packet.
        last_offset = data_offset + data_len;
        // If this is the last packet in the block, time to wrap it up.
        if hdr.tp_next_offset == 0 {
            break;
        }
        // Otherwise, move to the next packet in the block.
        base = base.add(hdr.tp_next_offset as usize);
    }
    // Tell the kernel: “Done reading, you may do your weird things now.”
    (*block).hdr.block_status = TP_STATUS_KERNEL;
    // Return the range of bytes that matter. Or None if we found nothing interesting.
    match first_offset {
        Some(start) => Some((start, last_offset)),
        None => None,
    }
}

#[derive(Debug)]
pub struct ParsedPacket<'a> {
    pub src_ip: Ipv4Addr,
    pub dst_ip: Ipv4Addr,
    pub src_port: u16,
    pub dst_port: u16,
    pub payload: &'a [u8],
}

pub fn parse_ipv4_tcp_packets(buffer: &[u8]) -> Vec<ParsedPacket<'_>> {
    // Pre-allocate like a responsible performance freak.
    let mut packets: Vec<ParsedPacket> = Vec::with_capacity(64);
    let mut offset: usize = 0;

    while offset + 14 + 20 <= buffer.len() {
        // Ethernet type field — time to gatekeep by protocol.
        let ethertype = u16::from_be_bytes([buffer[offset + 12], buffer[offset + 13]]);
        if ethertype != 0x0800 {
            // Not IPv4? Not interested. Skip like a bad playlist track.
            offset += 64; // Magic number alert: jump ahead generously.
            continue;
        }

        let ip_start: usize = offset + 14; // Ethernet header is 14 bytes — ancient networking law.
        if ip_start + 20 > buffer.len() {
            break; // Incomplete IP header = instant nope.
        }
        // IHL = Internet Header Length, measured in 32-bit words. Because why be normal?
        let ihl: usize = (buffer[ip_start] & 0x0F) as usize;
        let ip_header_len: usize = ihl * 4;
        let total_len: usize = u16::from_be_bytes([buffer[ip_start + 2], buffer[ip_start + 3]]) as usize;
        if ip_start + total_len > buffer.len() {
            break; // Entire IP packet doesn't fit? Denied.
        }

        let protocol: u8 = buffer[ip_start + 9];
        if protocol != 6 {
            // Not TCP. We only accept streams of suffering.
            offset += 14 + total_len;
            continue;
        }
        // Source and destination IP — IPv4 style, good old days.
        let src_ip: Ipv4Addr = Ipv4Addr::new(
            buffer[ip_start + 12],
            buffer[ip_start + 13],
            buffer[ip_start + 14],
            buffer[ip_start + 15],
        );

        let dst_ip: Ipv4Addr = Ipv4Addr::new(
            buffer[ip_start + 16],
            buffer[ip_start + 17],
            buffer[ip_start + 18],
            buffer[ip_start + 19],
        );
        // TCP starts right after the IP header (if the gods of offset smile upon us).
        let tcp_start: usize = ip_start + ip_header_len;
        if tcp_start + 20 > buffer.len() {
            break; // Minimum TCP header doesn't fit. Time to bail.
        }
        // Source and destination ports. These decide who gets yelled at.
        let src_port: u16 = u16::from_be_bytes([buffer[tcp_start], buffer[tcp_start + 1]]);
        let dst_port: u16 = u16::from_be_bytes([buffer[tcp_start + 2], buffer[tcp_start + 3]]);
        // TCP data offset — how long the TCP header really is (in 32-bit words, again. sigh).
        let data_offset: usize = ((buffer[tcp_start + 12] >> 4) * 4) as usize;
        let payload_start: usize = tcp_start + data_offset;
        if payload_start > buffer.len() {
            break; // Header says "more data" but buffer says "nope".
        }
        let payload_end: usize = ip_start + total_len;
        if payload_end > buffer.len() || payload_start > payload_end {
            break; // Sanity check: don't go out of bounds or wrap backwards in time.
        }
        let payload: &[u8] = &buffer[payload_start..payload_end];
        // Congrats, you have a real TCP packet. Put it in the bag.
        packets.push(ParsedPacket {
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            payload,
        });
        // Move to the next Ethernet frame. Repeat the dance.
        offset += 14 + total_len;
    }
    packets
}
