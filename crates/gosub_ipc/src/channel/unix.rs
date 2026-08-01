//! Unix transport backend: one `socketpair(2)`.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::UnixStream;

/// One end of a connected duplex link.
pub struct Channel(UnixStream);

/// Send half. Framing is done by [`crate::endpoint`]; this is just the byte sink.
pub type Tx = UnixStream;
/// Receive half.
pub type Rx = UnixStream;

impl Channel {
    /// A connected pair: one end for the engine, one to hand to the child.
    pub fn pair() -> io::Result<(Channel, Channel)> {
        let (a, b) = UnixStream::pair()?;
        Ok((Channel(a), Channel(b)))
    }

    /// Split into independent halves. Both are the same socket, so a shutdown
    /// or peer death is observed by each.
    pub fn split(self) -> io::Result<(Tx, Rx)> {
        let tx = self.0.try_clone()?;
        Ok((tx, self.0))
    }

    /// The child's end as an argv token: just the descriptor number, which is
    /// not a secret (the *inheritance* is what authenticates the link).
    pub fn to_argv(&self) -> String {
        self.0.as_raw_fd().to_string()
    }

    /// Adopt a channel this process inherited across `exec`.
    ///
    /// # Safety
    /// `spec` must be a token produced by [`Channel::to_argv`] in the parent,
    /// naming a descriptor this process inherited and does not otherwise own.
    pub unsafe fn from_argv(spec: &str) -> io::Result<Channel> {
        let fd: RawFd = spec
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad link fd"))?;
        Ok(Channel(UnixStream::from_raw_fd(fd)))
    }

    /// Let this end survive `exec` into the child, by clearing `FD_CLOEXEC`.
    ///
    /// Must be called from `pre_exec` (post-`fork`, in the child): doing it in
    /// the parent would leak this fd into every other concurrent spawn too.
    /// Async-signal-safe — two `fcntl` calls, nothing else.
    pub fn make_inheritable(fd: RawFd) -> io::Result<()> {
        // SAFETY: F_GETFD/F_SETFD on a descriptor the caller owns.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// The raw handle(s) `make_inheritable` must be applied to, captured so a
    /// `pre_exec` closure can use them without holding the `Channel` itself.
    pub fn raw(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    /// Wrap an already-connected stream (the Linux fork server receives its
    /// renderers' ends over `SCM_RIGHTS` rather than creating them).
    #[cfg(target_os = "linux")]
    pub fn from_stream(stream: UnixStream) -> Channel {
        Channel(stream)
    }
}
