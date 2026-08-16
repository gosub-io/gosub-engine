//! Shared-memory ring buffer for streaming large fetch bodies (Linux):
//! the net component (producer) keeps writing while the renderer (consumer)
//! keeps reading, wrapping at the end of a fixed window - pipe semantics
//! without the kernel copy. Unlike `shm`'s sealed tiles the contents are not
//! immutable (the producer keeps writing), so the consumer relies on cursor
//! validation instead of seals.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Header page: cursors + status, each on its own cache line. The data area
/// starts right after, so it's page-aligned.
const HEADER_LEN: usize = 4096;
const OFF_WRITE_POS: usize = 0;
const OFF_READ_POS: usize = 64;
const OFF_DONE: usize = 128;

/// `done` states: streaming (producer still writing), finished (all bytes
/// delivered), aborted (producer gave up - consumer must error, not wait).
const DONE_STREAMING: u32 = 0;
const DONE_FINISHED: u32 = 1;
const DONE_ABORTED: u32 = 2;

/// How long either side tolerates zero progress from its peer before giving
/// up. Bounds what a dead - or deliberately stalling - peer can cost.
const STALL_TIMEOUT: Duration = Duration::from_secs(5);
/// Individual futex waits are short slices of the stall budget, so a lost
/// wakeup costs at most one slice.
const WAIT_SLICE: Duration = Duration::from_millis(100);

/// Ring window bounds for the consumer's validation.
const MAX_CAPACITY: u32 = 64 * 1024 * 1024;
/// Cap on a *claimed* body length - bounds the consumer's output allocation,
/// same idea as `shm::MAX_TILE_DIM` (a renderer's rlimit would stop it anyway,
/// but refuse absurd claims before allocating).
pub const MAX_BODY_LEN: u64 = 128 * 1024 * 1024;

const REQUIRED_SEALS: libc::c_int = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW;

fn corrupt(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("ring protocol violation: {what}"))
}

/// Wait until `word` moves away from `expected`, one slice at a time.
/// `Ok(())` means "recheck" (woken, value moved, signal, or slice elapsed);
/// the caller owns the overall stall deadline.
fn futex_wait_slice(word: &AtomicU32, expected: u32) -> io::Result<()> {
    let ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: WAIT_SLICE.subsec_nanos() as libc::c_long + WAIT_SLICE.as_secs() as libc::c_long * 1_000_000_000,
    };
    // SAFETY: `word` points into a live shared mapping. Plain FUTEX_WAIT (no
    // PRIVATE flag): waiter and waker are different processes.
    let r = unsafe { libc::syscall(libc::SYS_futex, word.as_ptr(), libc::FUTEX_WAIT, expected, &ts, 0, 0) };
    if r == 0 {
        return Ok(());
    }
    match io::Error::last_os_error().raw_os_error() {
        // Value already moved / interrupted / slice elapsed: recheck.
        Some(libc::EAGAIN) | Some(libc::EINTR) | Some(libc::ETIMEDOUT) => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
}

fn futex_wake(word: &AtomicU32) {
    // SAFETY: as in futex_wait_slice.
    unsafe { libc::syscall(libc::SYS_futex, word.as_ptr(), libc::FUTEX_WAKE, i32::MAX, 0, 0, 0) };
}

/// A mapped ring (header + data window), shared by both sides' views.
struct RingMap {
    base: NonNull<u8>,
    len: usize,
}

// SAFETY: all shared-state access goes through the atomics; data copies are
// bounded by the validated cursor protocol.
unsafe impl Send for RingMap {}

impl RingMap {
    fn map(fd: &OwnedFd, len: usize) -> io::Result<RingMap> {
        // SAFETY: len was validated against the fd's (seal-fixed) size.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        // MAP_FAILED is handled above, so null is impossible - reported rather
        // than asserted, since this crate must not panic.
        let base = NonNull::new(ptr.cast()).ok_or_else(|| io::Error::other("mmap returned null"))?;
        Ok(RingMap { base, len })
    }

    fn capacity(&self) -> u32 {
        (self.len - HEADER_LEN) as u32
    }

    fn word(&self, off: usize) -> &AtomicU32 {
        // SAFETY: `off` is a 4-aligned offset inside the header page; an
        // AtomicU32 view of shared memory is exactly what cross-process
        // cursors + futexes require.
        unsafe { AtomicU32::from_ptr(self.base.as_ptr().add(off).cast()) }
    }

    fn write_pos(&self) -> &AtomicU32 {
        self.word(OFF_WRITE_POS)
    }

    fn read_pos(&self) -> &AtomicU32 {
        self.word(OFF_READ_POS)
    }

    fn done(&self) -> &AtomicU32 {
        self.word(OFF_DONE)
    }

    fn data(&self) -> *mut u8 {
        // SAFETY: the mapping is at least HEADER_LEN + 1 bytes long.
        unsafe { self.base.as_ptr().add(HEADER_LEN) }
    }
}

impl Drop for RingMap {
    fn drop(&mut self) {
        // SAFETY: exactly the range mmap returned; mapped once, unmapped once.
        unsafe { libc::munmap(self.base.as_ptr().cast(), self.len) };
    }
}

/// Producer side (the net component). Create once per stream, `write_all`
/// as bytes arrive, `finish()` on success - dropping without `finish` marks
/// the stream aborted so the consumer errors out instead of waiting.
pub struct RingProducer {
    map: RingMap,
    finished: bool,
}

impl RingProducer {
    /// Create the ring memfd and return the producer plus the fd to pass to
    /// the consumer (the caller sends it and drops its copy; the mapping -
    /// not the fd - keeps the producer's side alive).
    pub fn create(capacity: u32) -> io::Result<(RingProducer, OwnedFd)> {
        if capacity == 0 || capacity > MAX_CAPACITY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ring capacity out of range",
            ));
        }
        let len = HEADER_LEN + capacity as usize;

        // SAFETY: plain libc calls; the raw fd is wrapped immediately.
        let raw = unsafe { libc::memfd_create(c"gosub-ring".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        if unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // Fix the size while the contents stay writable: SHRINK/GROW don't
        // require writers to be gone (only F_SEAL_WRITE does). F_SEAL_SEAL
        // stops the consumer from adding seals that could sabotage us.
        let seals = REQUIRED_SEALS | libc::F_SEAL_SEAL;
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, seals) } < 0 {
            return Err(io::Error::last_os_error());
        }

        let map = RingMap::map(&fd, len)?; // memfd pages start zeroed
        Ok((RingProducer { map, finished: false }, fd))
    }

    /// Append the whole buffer, blocking (bounded) while the ring is full -
    /// the backpressure that stalls the producer instead of growing anyone's
    /// memory. Errors on a corrupt read cursor or a consumer that makes no
    /// progress within [`STALL_TIMEOUT`].
    pub fn write_all(&mut self, mut src: &[u8]) -> io::Result<()> {
        let cap = self.map.capacity();
        let mut deadline = Instant::now() + STALL_TIMEOUT;
        while !src.is_empty() {
            let w = self.map.write_pos().load(Ordering::Relaxed); // ours
            let r = self.map.read_pos().load(Ordering::Acquire); // theirs: local copy, then validate
            let used = w.wrapping_sub(r);
            if used > cap {
                return Err(corrupt("read cursor ran past the write cursor"));
            }
            let free = cap - used;
            if free == 0 {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "consumer stalled"));
                }
                futex_wait_slice(self.map.read_pos(), r)?;
                continue;
            }
            let n = (free as usize).min(src.len());
            let off = (w % cap) as usize;
            let first = n.min(cap as usize - off);
            // SAFETY: both segments lie inside the data window; the validated
            // cursor protocol guarantees they are in the free region, which
            // the consumer is not reading.
            unsafe {
                std::ptr::copy_nonoverlapping(src.as_ptr(), self.map.data().add(off), first);
                std::ptr::copy_nonoverlapping(src.as_ptr().add(first), self.map.data(), n - first);
            }
            self.map.write_pos().store(w.wrapping_add(n as u32), Ordering::Release);
            futex_wake(self.map.write_pos());
            src = &src[n..];
            deadline = Instant::now() + STALL_TIMEOUT; // progress resets patience
        }
        Ok(())
    }

    /// Mark the stream complete (all promised bytes written).
    pub fn finish(mut self) {
        self.finished = true;
        // Drop runs next and publishes DONE_FINISHED.
    }
}

impl Drop for RingProducer {
    fn drop(&mut self) {
        let status = if self.finished { DONE_FINISHED } else { DONE_ABORTED };
        self.map.done().store(status, Ordering::Release);
        // Wake a consumer waiting for data so it sees the verdict now.
        futex_wake(self.map.write_pos());
    }
}

/// Consumer side (the renderer): validate the received ring fd, then drain
/// exactly `body_len` bytes from it as the producer streams.
pub fn consume(fd: OwnedFd, body_len: u64) -> io::Result<Vec<u8>> {
    if body_len > MAX_BODY_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("claimed body length {body_len} exceeds the {MAX_BODY_LEN}-byte cap"),
        ));
    }

    // SAFETY: fcntl/fstat on an fd we own.
    let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
    if seals < 0 {
        return Err(io::Error::last_os_error()); // not a sealable memfd at all
    }
    if seals & REQUIRED_SEALS != REQUIRED_SEALS {
        return Err(corrupt("ring fd is not size-sealed (sender could shrink it)"));
    }
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let size = st.st_size as u128;
    if size <= HEADER_LEN as u128 || size > (HEADER_LEN as u128 + MAX_CAPACITY as u128) {
        return Err(corrupt("ring fd size out of range"));
    }
    let map = RingMap::map(&fd, size as usize)?;
    drop(fd); // the mapping keeps the pages alive
    let cap = map.capacity();

    let mut out = vec![0u8; body_len as usize];
    let mut got = 0usize;
    let mut deadline = Instant::now() + STALL_TIMEOUT;
    while got < out.len() {
        let r = map.read_pos().load(Ordering::Relaxed); // ours
        let w = map.write_pos().load(Ordering::Acquire); // theirs: local copy, then validate
        let avail = w.wrapping_sub(r);
        if avail > cap {
            return Err(corrupt("write cursor claims more than the ring holds"));
        }
        if avail == 0 {
            match map.done().load(Ordering::Acquire) {
                DONE_STREAMING => {
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "producer stalled"));
                    }
                    futex_wait_slice(map.write_pos(), w)?;
                    continue;
                }
                DONE_FINISHED => return Err(corrupt("stream finished short of the promised length")),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "producer aborted the stream",
                    ))
                }
            }
        }
        let n = (avail as usize).min(out.len() - got);
        let off = (r % cap) as usize;
        let first = n.min(cap as usize - off);
        // SAFETY: both segments lie inside the data window and target the
        // Vec's spare room. Single-pass: these bytes are copied out exactly
        // once, then the cursor advances and they are never read again.
        unsafe {
            std::ptr::copy_nonoverlapping(map.data().add(off), out.as_mut_ptr().add(got), first);
            std::ptr::copy_nonoverlapping(map.data(), out.as_mut_ptr().add(got + first), n - first);
        }
        got += n;
        map.read_pos().store(r.wrapping_add(n as u32), Ordering::Release);
        futex_wake(map.read_pos());
        deadline = Instant::now() + STALL_TIMEOUT; // progress resets patience
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(i: usize) -> u8 {
        (i.wrapping_mul(131) ^ (i >> 7)) as u8
    }

    /// A body 256× the ring: forces hundreds of wraps plus real full-ring /
    /// empty-ring waits between two threads.
    #[test]
    fn wraps_and_roundtrips() {
        let (mut producer, fd) = RingProducer::create(4096).unwrap();
        let body_len = 1024 * 1024usize;
        let feeder = std::thread::spawn(move || {
            let mut chunk = [0u8; 1000]; // deliberately no divisor of 4096
            let mut sent = 0usize;
            while sent < body_len {
                let n = chunk.len().min(body_len - sent);
                for (i, b) in chunk[..n].iter_mut().enumerate() {
                    *b = pattern(sent + i);
                }
                producer.write_all(&chunk[..n]).unwrap();
                sent += n;
            }
            producer.finish();
        });
        let body = consume(fd, body_len as u64).unwrap();
        feeder.join().unwrap();
        assert_eq!(body.len(), body_len);
        assert!(body.iter().enumerate().all(|(i, &b)| b == pattern(i)));
    }

    #[test]
    fn aborted_stream_is_an_error() {
        let (mut producer, fd) = RingProducer::create(8192).unwrap();
        producer.write_all(&[7u8; 100]).unwrap();
        drop(producer); // no finish(): abort
        assert!(consume(fd, 1024).is_err());
    }

    #[test]
    fn truncated_stream_is_an_error() {
        let (mut producer, fd) = RingProducer::create(8192).unwrap();
        producer.write_all(&[7u8; 100]).unwrap();
        producer.finish(); // done — but only 100 of the promised 1024 bytes
        assert!(consume(fd, 1024).is_err());
    }

    #[test]
    fn unsealed_fd_and_oversized_claim_refused() {
        // Unsealed fd: its size could change under the consumer.
        let raw = unsafe { libc::memfd_create(c"unsealed-ring".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
        assert!(raw >= 0);
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let len = (HEADER_LEN + 4096) as libc::off_t;
        assert_eq!(unsafe { libc::ftruncate(fd.as_raw_fd(), len) }, 0);
        assert!(consume(fd, 16).is_err());

        // Absurd claimed length: refused before anything is allocated.
        let (_producer, fd) = RingProducer::create(4096).unwrap();
        assert!(consume(fd, MAX_BODY_LEN + 1).is_err());
    }

    /// A malicious producer publishing a write cursor beyond the window must
    /// be detected as corruption, not read out of bounds.
    #[test]
    fn corrupt_write_cursor_detected() {
        let (producer, fd) = RingProducer::create(4096).unwrap();
        producer.map.write_pos().store(4097, Ordering::Release); // > capacity
        let err = consume(fd, 16).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{err}");
        drop(producer);
    }
}
