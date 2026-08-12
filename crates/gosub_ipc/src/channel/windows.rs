//! Windows transport backend: a pair of anonymous pipes.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{SetHandleInformation, ERROR_OPERATION_ABORTED, HANDLE, HANDLE_FLAG_INHERIT};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::IO::CancelIoEx;

/// One end of a duplex link: the read end of one pipe, the write end of the
/// other.
pub struct Channel {
    rx: File,
    tx: File,
}

/// Send half: the write end of one pipe, with an optional per-write deadline.
pub struct Tx {
    file: File,
    timeout: Option<Duration>,
}

/// Receive half: the read end of one pipe, with an optional per-read deadline.
pub struct Rx {
    file: File,
    timeout: Option<Duration>,
}

impl Tx {
    /// Bound how long a subsequent `write` may block; `None` clears it. See
    /// [`with_deadline`] for the `CancelIoEx` watchdog this arms.
    pub fn set_timeout(&mut self, dur: Option<Duration>) {
        self.timeout = dur;
    }
}

impl Rx {
    /// Bound how long a subsequent `read` may block; `None` clears it.
    pub fn set_timeout(&mut self, dur: Option<Duration>) {
        self.timeout = dur;
    }
}

impl Write for Tx {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        with_deadline(&self.file, self.timeout, |f| (&*f).write(buf))
    }
    fn flush(&mut self) -> io::Result<()> {
        (&self.file).flush()
    }
}

impl Read for Rx {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        with_deadline(&self.file, self.timeout, |f| (&*f).read(buf))
    }
}

/// A raw `HANDLE` we promise to only pass to [`CancelIoEx`] from the watchdog
/// thread. `HANDLE` is a raw pointer and thus `!Send`; the value is a kernel
/// handle, not a dereferenced address, so moving it across threads is sound.
struct SendHandle(HANDLE);
// SAFETY: only ever consumed by CancelIoEx, which takes a handle value.
unsafe impl Send for SendHandle {}

impl SendHandle {
    /// Cancel any pending synchronous I/O on this handle for the current
    /// process. Taking `&self` also forces the closure below to capture the
    /// whole `SendHandle` (not the bare `*mut c_void` field, which is `!Send`).
    fn cancel(&self) {
        // SAFETY: a handle owned by the caller's live `File`; a null OVERLAPPED
        // cancels all outstanding I/O on it for this process.
        unsafe { CancelIoEx(self.0, std::ptr::null()) };
    }
}

/// Run a blocking pipe operation with an optional deadline.
///
/// With `None`, calls `op` directly - no thread, no overhead. With `Some(dur)`,
/// a watchdog thread waits out the deadline and, if `op` is still blocked in
/// `ReadFile`/`WriteFile`, calls [`CancelIoEx`] on the handle to abort it (then
/// keeps cancelling every 20 ms until `op` returns, in case `op` was between two
/// syscalls when the deadline first passed). An aborted operation surfaces as
/// [`io::ErrorKind::TimedOut`].
fn with_deadline<T>(file: &File, timeout: Option<Duration>, op: impl FnOnce(&File) -> io::Result<T>) -> io::Result<T> {
    let Some(dur) = timeout else {
        return op(file);
    };

    let handle = SendHandle(file.as_raw_handle() as HANDLE);
    // `op` finishing drops `done_tx`, which disconnects the channel and wakes
    // the watchdog; the watchdog otherwise fires on the deadline.
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let watchdog = thread::spawn(move || {
        match done_rx.recv_timeout(dur) {
            // Op finished within the deadline (Disconnected), or an unexpected
            // send - either way, nothing to cancel.
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        loop {
            // `handle` outlives this thread - the caller joins before returning,
            // keeping its `File` (and thus the handle) alive throughout.
            handle.cancel();
            match done_rx.recv_timeout(Duration::from_millis(20)) {
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                _ => break,
            }
        }
    });

    let result = op(file);
    drop(done_tx); // signal completion; the watchdog stops cancelling
    let _ = watchdog.join();

    match result {
        Err(e) if e.raw_os_error() == Some(ERROR_OPERATION_ABORTED as i32) => {
            Err(io::Error::new(io::ErrorKind::TimedOut, "pipe operation timed out"))
        }
        other => other,
    }
}

/// Create one anonymous pipe, returning `(read, write)`.
fn pipe() -> io::Result<(File, File)> {
    let mut read: HANDLE = std::ptr::null_mut();
    let mut write: HANDLE = std::ptr::null_mut();
    // SAFETY: both out-params are valid; null attributes = default security,
    // non-inheritable; 0 = default buffer size.
    let ok = unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreatePipe succeeded, so both handles are valid and owned by us.
    unsafe {
        Ok((
            File::from_raw_handle(read as RawHandle),
            File::from_raw_handle(write as RawHandle),
        ))
    }
}

impl Channel {
    /// A connected pair: one end for the engine, one to hand to the child.
    pub fn pair() -> io::Result<(Channel, Channel)> {
        // Pipe A carries child → engine, pipe B carries engine → child.
        let (a_read, a_write) = pipe()?;
        let (b_read, b_write) = pipe()?;
        Ok((
            Channel {
                rx: a_read,
                tx: b_write,
            },
            Channel {
                rx: b_read,
                tx: a_write,
            },
        ))
    }

    /// Split into independent halves - already separate objects here, so
    /// unlike the Unix backend this cannot fail for want of a `dup`. Both halves
    /// start with no deadline armed (see [`Tx::set_timeout`]/[`Rx::set_timeout`]).
    pub fn split(self) -> io::Result<(Tx, Rx)> {
        Ok((
            Tx {
                file: self.tx,
                timeout: None,
            },
            Rx {
                file: self.rx,
                timeout: None,
            },
        ))
    }

    /// The child's end as an argv token: `read,write`. Handle *values* are not
    /// secrets, and an inherited handle keeps the same numeric value in the
    /// child, so the child can adopt them directly.
    pub fn to_argv(&self) -> String {
        format!(
            "{},{}",
            self.rx.as_raw_handle() as isize,
            self.tx.as_raw_handle() as isize
        )
    }

    /// Adopt a channel this process inherited from its parent.
    ///
    /// # Safety
    /// `spec` must be a token produced by [`Channel::to_argv`] in the parent,
    /// naming handles this process inherited and does not otherwise own.
    pub unsafe fn from_argv(spec: &str) -> io::Result<Channel> {
        let bad = || io::Error::new(io::ErrorKind::InvalidInput, "bad link handles");
        let (rx, tx) = spec.split_once(',').ok_or_else(bad)?;
        let rx: isize = rx.parse().map_err(|_| bad())?;
        let tx: isize = tx.parse().map_err(|_| bad())?;
        Ok(Channel {
            rx: File::from_raw_handle(rx as RawHandle),
            tx: File::from_raw_handle(tx as RawHandle),
        })
    }

    /// Let this end survive into the child, by setting `HANDLE_FLAG_INHERIT`.
    pub fn make_inheritable(handles: (RawHandle, RawHandle)) -> io::Result<()> {
        for h in [handles.0, handles.1] {
            // SAFETY: a handle the caller owns; the flag mask and value match.
            let ok = unsafe { SetHandleInformation(h as HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// The raw handles `make_inheritable` applies to, captured so the spawner
    /// can grant inheritance while the `Channel` still owns them.
    pub fn raw(&self) -> (RawHandle, RawHandle) {
        (self.rx.as_raw_handle(), self.tx.as_raw_handle())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    // Bytes plentifully exceeding the default anonymous-pipe buffer, so a peer
    // that never drains will make the writer block partway through.
    const OVERFLOW: usize = 1 << 20;

    #[test]
    fn round_trips_without_a_deadline() {
        let (a, b) = Channel::pair().unwrap();
        let (mut atx, _arx) = a.split().unwrap();
        let (_btx, mut brx) = b.split().unwrap();

        atx.write_all(b"hello").unwrap();
        atx.flush().unwrap();
        let mut buf = [0u8; 5];
        brx.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");
    }

    #[test]
    fn write_times_out_when_the_peer_never_reads() {
        // `_b` keeps the read end open (so writes block rather than break); it is
        // deliberately never read.
        let (a, _b) = Channel::pair().unwrap();
        let (mut tx, _arx) = a.split().unwrap();
        tx.set_timeout(Some(Duration::from_millis(200)));

        let buf = vec![0u8; OVERFLOW];
        let start = Instant::now();
        // `write_all` fills the pipe buffer, then blocks on the next chunk until
        // the watchdog cancels it - surfacing as TimedOut.
        let err = tx.write_all(&buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        // Cancelled promptly, not left to block indefinitely.
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn read_times_out_when_the_peer_never_writes() {
        // `_b` keeps the write end open, but never writes.
        let (a, _b) = Channel::pair().unwrap();
        let (_atx, mut rx) = a.split().unwrap();
        rx.set_timeout(Some(Duration::from_millis(200)));

        let mut buf = [0u8; 16];
        let start = Instant::now();
        let err = rx.read(&mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn clearing_the_deadline_restores_blocking_reads() {
        // With no deadline, a completed write is read back even though a timeout
        // was briefly armed and then cleared.
        let (a, b) = Channel::pair().unwrap();
        let (mut atx, _arx) = a.split().unwrap();
        let (_btx, mut brx) = b.split().unwrap();

        brx.set_timeout(Some(Duration::from_millis(200)));
        brx.set_timeout(None);
        atx.write_all(b"ok").unwrap();
        atx.flush().unwrap();
        let mut buf = [0u8; 2];
        brx.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ok");
    }
}
