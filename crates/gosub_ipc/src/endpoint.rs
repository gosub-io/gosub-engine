//! Length-framed bincode messaging: every frame is a little-endian u32 payload
//! length followed by the bincode-encoded message.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io;
#[cfg(feature = "multi-process")]
use std::io::{Read, Write};
// fd passing is `SCM_RIGHTS`-specific and only the Linux shared-memory paths
// (sealed tiles, the body ring) use it - macOS and Windows never send a
// descriptor, so this stays Linux-gated.
#[cfg(feature = "multi-process")]
use crate::channel;
#[cfg(all(feature = "multi-process", unix))]
use std::os::fd::AsRawFd;
#[cfg(all(feature = "multi-process", target_os = "linux"))]
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::sync::mpsc::{self, Receiver, Sender};

/// A corrupted (or malicious) length prefix must not make the peer allocate
/// unbounded memory.
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

/// Send half of an IPC link.
pub enum EndpointTx {
    #[cfg(feature = "multi-process")]
    Socket(channel::Tx),
    Local(Sender<Vec<u8>>),
}

/// Receive half of an IPC link.
pub enum EndpointRx {
    #[cfg(feature = "multi-process")]
    Socket(channel::Rx),
    Local(Receiver<Vec<u8>>),
}

impl EndpointTx {
    /// Whether this endpoint can carry file descriptors (`SCM_RIGHTS`) - true
    /// for the socket transport, false for in-process channels, where shared
    /// memory would be pointless anyway (same address space). Only the
    /// shared-memory tile path asks (hence the cfg).
    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    pub fn supports_fd_passing(&self) -> bool {
        matches!(self, EndpointTx::Socket(_))
    }

    /// Pass a duplicate of `fd` to the peer. The caller keeps (and should
    /// promptly close) its own copy.
    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    pub fn send_fd(&mut self, fd: RawFd) -> io::Result<()> {
        match self {
            // SAFETY: both descriptors are valid - the stream is live and the
            // caller owns `fd`.
            EndpointTx::Socket(stream) => unsafe { send_fd(stream.as_raw_fd(), fd) },
            EndpointTx::Local(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no fd passing on local channels",
            )),
        }
    }

    /// Bound how long a subsequent `send` will block trying to hand bytes to
    /// the peer: `Some(dur)` arms a per-write timeout on the underlying socket
    /// so a peer that refuses to read (a full socket buffer) is abandoned
    /// (send returns `WouldBlock`/`TimedOut`) rather than blocking the caller
    /// forever; `None` clears it.
    #[cfg_attr(not(feature = "multi-process"), allow(unused_variables))]
    pub fn set_write_timeout(&mut self, dur: Option<std::time::Duration>) -> io::Result<()> {
        match self {
            #[cfg(all(feature = "multi-process", unix))]
            EndpointTx::Socket(stream) => stream.set_write_timeout(dur),
            #[cfg(all(feature = "multi-process", windows))]
            EndpointTx::Socket(stream) => {
                stream.set_timeout(dur);
                Ok(())
            }
            EndpointTx::Local(_) => Ok(()),
        }
    }

    pub fn send<T: Serialize>(&mut self, msg: &T) -> io::Result<()> {
        match self {
            #[cfg(feature = "multi-process")]
            EndpointTx::Socket(stream) => send_msg(stream, msg),
            EndpointTx::Local(tx) => {
                let payload = bincode::serialize(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                if payload.len() > MAX_FRAME_LEN as usize {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
                }
                tx.send(payload)
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "peer gone"))
            }
        }
    }
}

impl EndpointRx {
    /// Receive a file descriptor the peer announced (e.g. right after a
    /// `TileShm` message). Fails on local channels, which never carry fds.
    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    pub fn recv_fd(&mut self) -> io::Result<OwnedFd> {
        match self {
            // SAFETY: the stream is a valid open descriptor.
            EndpointRx::Socket(stream) => unsafe { recv_fd(stream.as_raw_fd()) },
            EndpointRx::Local(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no fd passing on local channels",
            )),
        }
    }

    /// Whether the peer is still there, without consuming anything: a Unix
    /// socket whose peer closed reads as end-of-file. `true` for a local
    /// channel (its peer is this process) and when the answer is unknown.
    #[cfg(all(feature = "multi-process", unix))]
    pub fn peer_alive(&self) -> bool {
        match self {
            EndpointRx::Socket(stream) => {
                let mut probe = [0u8; 1];
                // SAFETY: the stream is a valid open descriptor and `probe`
                // is a valid 1-byte buffer; MSG_PEEK leaves the data in place,
                // MSG_DONTWAIT makes the call non-blocking.
                let n = unsafe {
                    libc::recv(
                        stream.as_raw_fd(),
                        probe.as_mut_ptr().cast(),
                        1,
                        libc::MSG_PEEK | libc::MSG_DONTWAIT,
                    )
                };
                if n > 0 {
                    return true;
                }
                if n == 0 {
                    return false;
                }
                matches!(
                    io::Error::last_os_error().kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                )
            }
            EndpointRx::Local(_) => true,
        }
    }

    /// Bound how long a subsequent `recv` will block waiting for bytes:
    /// `Some(dur)` arms a per-read timeout on the underlying socket so a peer
    /// that stops sending is abandoned (recv returns `WouldBlock`/`TimedOut`)
    /// rather than blocking the caller forever; `None` clears it.
    #[cfg_attr(not(feature = "multi-process"), allow(unused_variables))]
    pub fn set_read_timeout(&mut self, dur: Option<std::time::Duration>) -> io::Result<()> {
        match self {
            #[cfg(all(feature = "multi-process", unix))]
            EndpointRx::Socket(stream) => stream.set_read_timeout(dur),
            #[cfg(all(feature = "multi-process", windows))]
            EndpointRx::Socket(stream) => {
                stream.set_timeout(dur);
                Ok(())
            }
            EndpointRx::Local(_) => Ok(()),
        }
    }

    pub fn recv<T: DeserializeOwned>(&mut self) -> io::Result<T> {
        match self {
            #[cfg(feature = "multi-process")]
            EndpointRx::Socket(stream) => recv_msg(stream),
            EndpointRx::Local(rx) => {
                let payload = rx
                    .recv()
                    .map_err(|_| io::Error::new(io::ErrorKind::UnexpectedEof, "peer gone"))?;
                bincode::deserialize(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
            }
        }
    }
}

/// One end of a duplex IPC link. Components are written against this type
/// only, so the exact same renderer/net code runs either as a child *process*
/// (Socket) or as an in-process *thread* (Local). Both variants carry
/// identical bincode frames, so the protocol - and every policy check built
/// on it - behaves the same in both modes.
pub struct Endpoint {
    pub tx: EndpointTx,
    pub rx: EndpointRx,
}

impl Endpoint {
    /// Wrap a connected transport channel, splitting it into halves that can
    /// be used independently (a reader thread plus the event loop's writer).
    #[cfg(feature = "multi-process")]
    pub fn from_channel(ch: channel::Channel) -> io::Result<Endpoint> {
        let (tx, rx) = ch.split()?;
        Ok(Endpoint {
            tx: EndpointTx::Socket(tx),
            rx: EndpointRx::Socket(rx),
        })
    }

    /// Adopt the link this process inherited across `exec`, named by the argv
    /// token the parent produced with [`channel::Channel::to_argv`].
    ///
    /// # Correct use
    #[cfg(feature = "multi-process")]
    pub fn adopt_inherited(spec: &str) -> io::Result<Endpoint> {
        // SAFETY: `from_argv` only requires that the token names a descriptor
        // this process owns and has not otherwise adopted.
        let channel = unsafe { channel::Channel::from_argv(spec)? };
        Endpoint::from_channel(channel)
    }

    pub fn send<T: Serialize>(&mut self, msg: &T) -> io::Result<()> {
        self.tx.send(msg)
    }

    pub fn recv<T: DeserializeOwned>(&mut self) -> io::Result<T> {
        self.rx.recv()
    }

    /// Split into independent send/receive halves (e.g. reader thread +
    /// event-loop writer).
    pub fn split(self) -> (EndpointTx, EndpointRx) {
        (self.tx, self.rx)
    }

    /// An endpoint over two descriptors of the same socket, for a confined
    /// process that may not `dup` (the sender passed the fd twice).
    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    pub fn from_halves(tx: std::os::unix::net::UnixStream, rx: std::os::unix::net::UnixStream) -> Endpoint {
        Endpoint {
            tx: EndpointTx::Socket(tx),
            rx: EndpointRx::Socket(rx),
        }
    }

    /// The descriptors behind this link, for a forked child that must close
    /// them; empty for an in-process channel.
    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    pub fn raw_fds(&self) -> Vec<RawFd> {
        let mut fds = Vec::new();
        if let EndpointTx::Socket(s) = &self.tx {
            fds.push(s.as_raw_fd());
        }
        if let EndpointRx::Socket(s) = &self.rx {
            fds.push(s.as_raw_fd());
        }
        fds
    }
}

/// A connected pair of in-process endpoints (single-process mode's stand-in
/// for `socketpair(2)`).
pub fn local_pair() -> (Endpoint, Endpoint) {
    let (tx_a, rx_b) = mpsc::channel();
    let (tx_b, rx_a) = mpsc::channel();
    (
        Endpoint {
            tx: EndpointTx::Local(tx_a),
            rx: EndpointRx::Local(rx_a),
        },
        Endpoint {
            tx: EndpointTx::Local(tx_b),
            rx: EndpointRx::Local(rx_b),
        },
    )
}

#[cfg(feature = "multi-process")]
pub fn send_msg<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    let payload = bincode::serialize(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len =
        u32::try_from(payload.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))?;
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&payload)?;
    w.flush()
}

#[cfg(feature = "multi-process")]
pub fn recv_msg<T: DeserializeOwned>(r: &mut impl Read) -> io::Result<T> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("refusing {len}-byte frame (corrupt or malicious length prefix)"),
        ));
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload)?;
    bincode::deserialize(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Send one file descriptor over a Unix socket via `SCM_RIGHTS`, with a 1-byte
/// dummy payload (fd-passing must carry at least one data byte). The kernel
/// duplicates the fd into the receiver; the sender keeps its own copy.
///
/// # Safety
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub unsafe fn send_fd(sock_fd: RawFd, fd: RawFd) -> io::Result<()> {
    let mut byte = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr().cast(),
        iov_len: 1,
    };
    let mut cmsg = [0u8; 32]; // > CMSG_SPACE(size_of::<RawFd>())

    let mut msg: libc::msghdr = std::mem::zeroed();
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg.as_mut_ptr().cast();
    msg.msg_controllen = libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as _;

    let cmsgp = libc::CMSG_FIRSTHDR(&msg);
    (*cmsgp).cmsg_level = libc::SOL_SOCKET;
    (*cmsgp).cmsg_type = libc::SCM_RIGHTS;
    (*cmsgp).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
    std::ptr::copy_nonoverlapping(&fd, libc::CMSG_DATA(cmsgp).cast::<RawFd>(), 1);

    if libc::sendmsg(sock_fd, &msg, 0) < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Receive one file descriptor sent via [`send_fd`]. The new fd is created
/// `CLOEXEC` (`MSG_CMSG_CLOEXEC`) and returned owned, so an early return in
/// the caller can never leak it.
///
/// # Safety
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub unsafe fn recv_fd(sock_fd: RawFd) -> io::Result<OwnedFd> {
    let mut byte = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr().cast(),
        iov_len: 1,
    };
    // Room for several fds on purpose: a smuggling attempt should be *seen*
    // (received, counted, closed) rather than silently truncated.
    let mut cmsg = [0u8; 64];

    let mut msg: libc::msghdr = std::mem::zeroed();
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg.as_mut_ptr().cast();
    msg.msg_controllen = cmsg.len() as _;

    #[cfg(target_os = "linux")]
    let flags = libc::MSG_CMSG_CLOEXEC;
    #[cfg(not(target_os = "linux"))]
    let flags = 0;
    let n = libc::recvmsg(sock_fd, &mut msg, flags);
    if n <= 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no fd received"));
    }

    // Adopt every fd across every SCM_RIGHTS cmsg (each may carry several)
    // before any verdict, so whatever is rejected below is closed, not leaked.
    let mut fds: Vec<OwnedFd> = Vec::new();
    let mut cmsgp = libc::CMSG_FIRSTHDR(&msg);
    while !cmsgp.is_null() {
        if (*cmsgp).cmsg_level == libc::SOL_SOCKET && (*cmsgp).cmsg_type == libc::SCM_RIGHTS {
            let payload = (*cmsgp).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
            let data = libc::CMSG_DATA(cmsgp).cast::<RawFd>();
            for i in 0..payload / std::mem::size_of::<RawFd>() {
                let mut fd: RawFd = -1;
                std::ptr::copy_nonoverlapping(data.add(i), &mut fd, 1);
                if fd >= 0 {
                    fds.push(OwnedFd::from_raw_fd(fd));
                }
            }
        }
        cmsgp = libc::CMSG_NXTHDR(&msg, cmsgp);
    }

    // Truncated control data = the peer attached more than even the roomy
    // buffer holds; the kernel closed the overflow, we close the rest here.
    if msg.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control data truncated (peer attached too many fds)",
        ));
    }
    if fds.len() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected exactly 1 fd in the hand-off, got {}", fds.len()),
        ));
    }
    fds.pop()
        .ok_or_else(|| io::Error::other("fd hand-off vanished after being counted"))
}

#[cfg(test)]
mod tests {
    use super::*;
    // The `SCM_RIGHTS` tests below drive a socketpair directly rather than
    // through `channel`, since what they pin down is fd-passing behaviour -
    // Linux-only, like the paths that use it.
    use serde::Deserialize;
    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    use std::os::unix::net::UnixStream;

    /// Stand-in for a caller's protocol: this crate frames arbitrary serde
    /// types, so the transport tests supply their own rather than depending on
    /// any one boundary's messages.
    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
    enum TestMsg {
        Shutdown,
        Reply {
            request_id: u64,
            status: u16,
            body: Vec<u8>,
        },
    }

    #[test]
    fn local_pair_roundtrip() {
        let (mut a, mut b) = local_pair();
        a.send(&TestMsg::Shutdown).unwrap();
        assert_eq!(b.recv::<TestMsg>().unwrap(), TestMsg::Shutdown);
    }

    #[cfg(feature = "multi-process")]
    #[test]
    fn frame_roundtrip() {
        let msg = TestMsg::Reply {
            request_id: 42,
            status: 200,
            body: vec![9, 9, 9],
        };
        let mut buf: Vec<u8> = Vec::new();
        send_msg(&mut buf, &msg).unwrap();
        let mut cur = std::io::Cursor::new(buf);
        let back: TestMsg = recv_msg(&mut cur).unwrap();
        assert_eq!(back, msg);
    }

    /// Hand-rolled sendmsg attaching `fds` to ONE SCM_RIGHTS cmsg - what a
    /// compromised peer (sendmsg is on its allowlist) can do to smuggle
    /// descriptors into the fd hand-off.
    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    unsafe fn send_fds_one_cmsg(sock_fd: std::os::fd::RawFd, fds: &[std::os::fd::RawFd]) {
        let mut byte = [0u8; 1];
        let mut iov = libc::iovec {
            iov_base: byte.as_mut_ptr().cast(),
            iov_len: 1,
        };
        let payload = std::mem::size_of_val(fds) as u32;
        let mut buf = vec![0u8; libc::CMSG_SPACE(payload) as usize];

        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = buf.as_mut_ptr().cast();
        msg.msg_controllen = buf.len() as _;

        let cmsgp = libc::CMSG_FIRSTHDR(&msg);
        (*cmsgp).cmsg_level = libc::SOL_SOCKET;
        (*cmsgp).cmsg_type = libc::SCM_RIGHTS;
        (*cmsgp).cmsg_len = libc::CMSG_LEN(payload) as _;
        std::ptr::copy_nonoverlapping(
            fds.as_ptr(),
            libc::CMSG_DATA(cmsgp).cast::<std::os::fd::RawFd>(),
            fds.len(),
        );
        assert!(libc::sendmsg(sock_fd, &msg, 0) >= 0, "{}", io::Error::last_os_error());
    }

    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    #[test]
    fn peer_alive_sees_a_closed_peer_without_consuming() {
        let (mine, peer) = UnixStream::pair().unwrap();
        let ep = Endpoint::from_channel(channel::Channel::from_stream(mine)).unwrap();
        let (_tx, mut rx) = ep.split();
        assert!(rx.peer_alive(), "a silent live peer is alive");

        let mut peer_ep = Endpoint::from_channel(channel::Channel::from_stream(peer)).unwrap();
        peer_ep.send(&TestMsg::Shutdown).unwrap();
        assert!(rx.peer_alive(), "a peer with a frame pending is alive");
        assert_eq!(
            rx.recv::<TestMsg>().unwrap(),
            TestMsg::Shutdown,
            "the probe consumed nothing"
        );

        drop(peer_ep);
        assert!(!rx.peer_alive(), "a closed peer is dead");
    }

    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    #[test]
    fn recv_fd_roundtrips_exactly_one() {
        let (a, b) = UnixStream::pair().unwrap();
        unsafe { send_fd(a.as_raw_fd(), 2).unwrap() }; // stderr as a stand-in
        let fd = unsafe { recv_fd(b.as_raw_fd()) }.unwrap();
        assert!(fd.as_raw_fd() >= 0);
    }

    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    #[test]
    fn recv_fd_rejects_smuggled_extra_fds() {
        // Several fds stuffed into the one-fd hand-off must be refused - and
        // the extras closed, not leaked into the receiver's fd table (the
        // OwnedFd-before-verdict adoption in recv_fd is what guarantees the
        // close; this pins the refusal).
        let (a, b) = UnixStream::pair().unwrap();
        unsafe { send_fds_one_cmsg(a.as_raw_fd(), &[2, 2, 2]) };
        assert!(unsafe { recv_fd(b.as_raw_fd()) }.is_err());
    }

    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    #[test]
    fn recv_fd_rejects_data_without_fd() {
        use std::io::Write;
        let (mut a, b) = UnixStream::pair().unwrap();
        a.write_all(&[0u8]).unwrap(); // plain byte, no SCM_RIGHTS attached
        assert!(unsafe { recv_fd(b.as_raw_fd()) }.is_err());
    }

    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    #[test]
    fn recv_returns_a_timeout_error_when_the_peer_is_silent() {
        // The primitive behind the decode stall timeout: with a read timeout
        // armed, recv on a socket whose peer sends nothing must return a
        // WouldBlock/TimedOut error near the deadline - never block forever.
        use std::time::{Duration, Instant};
        // Keep `_peer` bound so the socket is not closed (a closed peer gives EOF,
        // not a timeout - a different, already-handled case).
        let (mine, _peer) = UnixStream::pair().unwrap();
        let ep = Endpoint::from_channel(channel::Channel::from_stream(mine)).unwrap();
        let (_tx, mut rx) = ep.split();
        rx.set_read_timeout(Some(Duration::from_millis(150))).unwrap();

        let start = Instant::now();
        let err = rx.recv::<u32>().expect_err("silent peer must not yield a value");
        let elapsed = start.elapsed();

        assert!(
            matches!(err.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut),
            "expected a timeout error, got {:?}",
            err.kind()
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "recv should return near the deadline, took {elapsed:?}"
        );
    }

    #[cfg(all(feature = "multi-process", target_os = "linux"))]
    #[test]
    fn send_returns_a_timeout_error_when_the_peer_never_reads() {
        // The primitive behind the reply-write timeout: with a write timeout
        // armed, sending to a peer that never drains eventually fills the socket
        // buffers, and the send must then return a WouldBlock/TimedOut error near
        // the deadline - never block forever, which on the single-threaded engine
        // loop with blocking writes would wedge the whole browser.
        use std::time::{Duration, Instant};
        // Keep `_peer` bound and never read from it: the buffers fill (a closed
        // peer would instead give a broken pipe - a different, already-handled
        // case).
        let (mine, _peer) = UnixStream::pair().unwrap();
        let mut ep = Endpoint::from_channel(channel::Channel::from_stream(mine)).unwrap();
        ep.tx.set_write_timeout(Some(Duration::from_millis(150))).unwrap();

        // A sizable payload so the fixed-size socket buffers fill in a bounded
        // number of iterations; the peer reads nothing, so a send must time out.
        let big = vec![0u8; 256 * 1024];
        let start = Instant::now();
        let mut err = None;
        for _ in 0..1024 {
            if let Err(e) = ep.send(&big) {
                err = Some(e);
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "send never blocked against a non-reading peer"
            );
        }
        let err = err.expect("a non-reading peer must eventually make send time out");
        assert!(
            matches!(err.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut),
            "expected a timeout error, got {:?}",
            err.kind()
        );
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "send should return near the deadline, took {:?}",
            start.elapsed()
        );
    }

    #[cfg(feature = "multi-process")]
    #[test]
    fn oversized_length_prefix_rejected() {
        // A corrupt/malicious length prefix must not force an allocation.
        let mut buf: Vec<u8> = (MAX_FRAME_LEN + 1).to_le_bytes().to_vec();
        buf.extend_from_slice(&[0u8; 8]);
        let mut cur = std::io::Cursor::new(buf);
        let r: io::Result<TestMsg> = recv_msg(&mut cur);
        assert!(r.is_err(), "should reject an oversized frame");
    }

    /// Deterministic stand-in for `cargo fuzz run ipc_frame`: hammer the
    /// untrusted → broker deserialization (the frames a compromised child sends
    /// in) with arbitrary bytes. Each must return Ok/Err, never panic or
    /// over-allocate. The `fuzz/` target explores far more; this is the CI floor.
    #[cfg(feature = "multi-process")]
    #[test]
    fn recv_msg_never_panics_on_arbitrary_frames() {
        let mut s = 0x1234_5678_9abc_def0u64;
        for _ in 0..50_000 {
            let len = (xorshift(&mut s) % 128) as usize;
            let buf: Vec<u8> = (0..len).map(|_| xorshift(&mut s) as u8).collect();
            let _ = recv_msg::<TestMsg>(&mut std::io::Cursor::new(&buf));
            let _ = recv_msg::<Vec<u8>>(&mut std::io::Cursor::new(&buf));
            let _ = recv_msg::<(u64, String)>(&mut std::io::Cursor::new(&buf));
        }
    }

    /// Tiny deterministic xorshift PRNG - reproducible, no `rand`, no clock seed.
    #[cfg(feature = "multi-process")]
    fn xorshift(s: &mut u64) -> u64 {
        let mut x = *s;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *s = x;
        x
    }
}
