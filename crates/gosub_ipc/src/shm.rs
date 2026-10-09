//! Shared-memory tile transport (Linux): the renderer rasterizes into a
//! `memfd`, seals it, and passes the *fd* to the engine over the existing
//! `SCM_RIGHTS` channel - the engine then maps the same physical pages
//! instead of copying ~1 MiB of pixels through the socket. Only a ~10-byte
//! header travels in-band. This is the channel OOPIFs and a future decode
//! process will reuse.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// RGBA8.
pub const BYTES_PER_PIXEL: usize = 4;

/// Upper bound on either tile dimension the consumer will accept. 2048² × 4
/// = 16 MiB - deliberately the same ceiling `MAX_FRAME_LEN` puts on an
/// in-band tile, so the shared-memory path never lets a renderer pin *more*
/// engine memory per message than the socket path already could; the
/// per-source gate then bounds how many such messages are in flight, exactly
/// as it does for copied tiles.
pub const MAX_TILE_DIM: u32 = 2048;

/// The seals a consumer must see before touching the pages: size fixed in
/// both directions, contents immutable.
const REQUIRED_SEALS: libc::c_int = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE;

/// Byte length of a `width`×`height` RGBA tile, refusing out-of-range
/// dimensions. Both sides use this: the producer to size the memfd, the
/// consumer to derive what the fd must hold *from the dimensions* - never
/// from a length claimed in a message.
fn tile_len(width: u32, height: u32) -> io::Result<usize> {
    if width == 0 || height == 0 || width > MAX_TILE_DIM || height > MAX_TILE_DIM {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("tile dimensions {width}x{height} out of range"),
        ));
    }
    Ok(width as usize * height as usize * BYTES_PER_PIXEL)
}

/// Producer side: create a sealed, immutable memfd holding one rendered tile,
/// ready to pass to the consumer. `fill` receives the zeroed pixel buffer.
pub fn create_sealed_tile(width: u32, height: u32, fill: impl FnOnce(&mut [u8])) -> io::Result<OwnedFd> {
    create_sealed(c"gosub-tile", tile_len(width, height)?, fill)
}

/// Largest blob [`create_sealed_blob`] makes and [`read_sealed_blob`] accepts: the
/// image decoders' input ceiling, so nothing past it could be used anyway, and twice
/// the fetcher's default body cap.
pub const MAX_BLOB_LEN: usize = 128 * 1024 * 1024;

/// Producer side: a sealed, immutable memfd holding `len` bytes (at most
/// [`MAX_BLOB_LEN`]), for a payload too large for one frame - a subresource body
/// on its way to a renderer, the tile channel in reverse. `fill` receives the
/// zeroed buffer.
pub fn create_sealed_blob(len: usize, fill: impl FnOnce(&mut [u8])) -> io::Result<OwnedFd> {
    if len == 0 || len > MAX_BLOB_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("blob of {len} bytes out of range"),
        ));
    }
    create_sealed(c"gosub-blob", len, fill)
}

/// Consumer side of [`create_sealed_blob`]: the blob's `len` bytes, read from the
/// start. Plain `read`s only - no `mmap`, `fstat` or `fcntl` - so a renderer's
/// syscall filter needs nothing it does not already allow. The sender is the
/// trusted side; `len` is checked against [`MAX_BLOB_LEN`] so a bug there fails
/// here rather than allocating without bound.
pub fn read_sealed_blob(fd: OwnedFd, len: usize) -> io::Result<Vec<u8>> {
    use std::io::Read as _;
    if len == 0 || len > MAX_BLOB_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("blob of {len} bytes out of range"),
        ));
    }
    let mut body = vec![0u8; len];
    std::fs::File::from(fd).read_exact(&mut body)?;
    Ok(body)
}

/// A sealed memfd of `len` bytes, filled by `fill`.
fn create_sealed(name: &std::ffi::CStr, len: usize, fill: impl FnOnce(&mut [u8])) -> io::Result<OwnedFd> {
    // SAFETY: plain libc calls on values we own; the raw fd is wrapped in an
    // OwnedFd immediately, so every early return below closes it.
    let raw = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };

    if unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) } < 0 {
        return Err(io::Error::last_os_error());
    }

    // Write the contents through a temporary mapping, then unmap: F_SEAL_WRITE
    // below is refused while any writable mapping exists.
    unsafe {
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd.as_raw_fd(),
            0,
        );
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        fill(std::slice::from_raw_parts_mut(ptr.cast::<u8>(), len));
        libc::munmap(ptr, len);
    }

    // Freeze size and contents, and seal the seals themselves. After this no
    // process - including this one - can modify it, so it is safe to hand out.
    let all = REQUIRED_SEALS | libc::F_SEAL_SEAL;
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, all) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

/// A consumer-side read-only view of a sealed tile. The backing fd is closed
/// as soon as the mapping exists (the mapping pins the pages); dropping the
/// mapping unmaps them.
pub struct TileMapping {
    ptr: std::ptr::NonNull<u8>,
    len: usize,
}

impl std::fmt::Debug for TileMapping {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TileMapping")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

// SAFETY: the mapping is PROT_READ over a memfd sealed with F_SEAL_WRITE (no
// writer can exist in any process) and F_SEAL_SHRINK (the range stays valid),
// so reading it from any thread is sound.
unsafe impl Send for TileMapping {}
unsafe impl Sync for TileMapping {}

impl TileMapping {
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: ptr/len describe a live PROT_READ mapping we own.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

// So a mapping can *become* a `bytes::Bytes` via `Bytes::from_owner` - the
// tile then flows through pixel-consuming code (`CachedTile.data`) with the
// mapping as its backing storage, unmapped when the last reference drops.
// Zero copies from the renderer's rasterizer to the compositor's blend.
impl AsRef<[u8]> for TileMapping {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Drop for TileMapping {
    fn drop(&mut self) {
        // SAFETY: exactly the range mmap returned; mapped once, unmapped once.
        unsafe { libc::munmap(self.ptr.as_ptr().cast(), self.len) };
    }
}

/// Consumer side: validate a received tile fd and map it read-only.
pub fn map_sealed_tile(fd: OwnedFd, width: u32, height: u32) -> io::Result<TileMapping> {
    let len = tile_len(width, height)?;

    // SAFETY: fcntl/fstat/mmap on an fd we own.
    let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
    if seals < 0 {
        return Err(io::Error::last_os_error()); // not a sealable memfd at all
    }
    if seals & REQUIRED_SEALS != REQUIRED_SEALS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "tile fd is not sealed (sender could still write or shrink it)",
        ));
    }

    // The real size, not the claimed one. F_SEAL_SHRINK (verified above) makes
    // this check stable: the file cannot shrink afterwards, so no read through
    // the mapping can SIGBUS. Exactly the tile's size, not at least: the
    // producer sizes the memfd to the tile, and a bigger one would keep its
    // slack (shmem the renderer's data limit does not count) pinned for as
    // long as this mapping lives, which is as long as the tile is kept.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } < 0 {
        return Err(io::Error::last_os_error());
    }
    if (st.st_size as u128) != len as u128 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("tile fd holds {} bytes, {width}x{height} is {len}", st.st_size),
        ));
    }

    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd.as_raw_fd(),
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    // mmap signals failure with MAP_FAILED, handled above, so a null here is
    // impossible - reported rather than asserted, since this crate must not panic.
    let ptr = std::ptr::NonNull::new(ptr.cast()).ok_or_else(|| io::Error::other("mmap returned null"))?;
    Ok(TileMapping { ptr, len })
    // `fd` drops (closes) here; the mapping keeps the pages alive.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_tile_roundtrip() {
        let fd = create_sealed_tile(8, 4, |buf| {
            for (i, b) in buf.iter_mut().enumerate() {
                *b = (i * 7) as u8;
            }
        })
        .unwrap();
        let map = map_sealed_tile(fd, 8, 4).unwrap();
        assert_eq!(map.as_slice().len(), 8 * 4 * BYTES_PER_PIXEL);
        assert!(map.as_slice().iter().enumerate().all(|(i, &b)| b == (i * 7) as u8));
    }

    #[test]
    fn unsealed_fd_refused() {
        // Same memfd, correct size - but never sealed. A consumer must refuse
        // it: the sender could still write to or shrink it after validation.
        let len = tile_len(8, 4).unwrap();
        let raw = unsafe { libc::memfd_create(c"unsealed".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
        assert!(raw >= 0);
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        assert_eq!(unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) }, 0);
        assert!(map_sealed_tile(fd, 8, 4).is_err());
    }

    #[test]
    fn undersized_fd_refused() {
        // Sealed as 8x4 but claimed as 512x512: the fstat check must catch
        // that the fd cannot hold the claimed tile.
        let fd = create_sealed_tile(8, 4, |_| {}).unwrap();
        assert!(map_sealed_tile(fd, 512, 512).is_err());
    }

    #[test]
    fn oversized_fd_refused() {
        // Sealed at 8x4 pixels' worth of bytes plus a megabyte of slack: the
        // slack would stay pinned behind a 4-byte-per-pixel mapping.
        let len = tile_len(8, 4).unwrap() + 1024 * 1024;
        let raw = unsafe { libc::memfd_create(c"oversized".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
        assert!(raw >= 0);
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        assert_eq!(unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) }, 0);
        let seals = REQUIRED_SEALS | libc::F_SEAL_SEAL;
        assert_eq!(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, seals) }, 0);
        assert!(map_sealed_tile(fd, 8, 4).is_err());
    }

    /// A blob reads back exactly, through `read` alone, and is sealed against change.
    #[test]
    fn sealed_blob_roundtrip() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let fd = create_sealed_blob(data.len(), |buf| buf.copy_from_slice(&data)).unwrap();
        let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
        assert_eq!(seals & REQUIRED_SEALS, REQUIRED_SEALS);
        assert_eq!(read_sealed_blob(fd, data.len()).unwrap(), data);
    }

    #[test]
    fn blob_lengths_out_of_range_refused() {
        assert!(create_sealed_blob(0, |_| {}).is_err());
        assert!(create_sealed_blob(MAX_BLOB_LEN + 1, |_| {}).is_err());
        let fd = create_sealed_blob(4, |buf| buf.copy_from_slice(b"abcd")).unwrap();
        assert!(read_sealed_blob(fd, MAX_BLOB_LEN + 1).is_err());
        // Claiming more than the fd holds fails rather than padding.
        let fd = create_sealed_blob(4, |buf| buf.copy_from_slice(b"abcd")).unwrap();
        assert!(read_sealed_blob(fd, 5).is_err());
    }

    #[test]
    fn absurd_dimensions_refused() {
        assert!(tile_len(0, 4).is_err());
        assert!(tile_len(MAX_TILE_DIM + 1, 1).is_err());
        assert!(create_sealed_tile(0, 0, |_| {}).is_err());
    }
}
