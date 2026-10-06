use std::ops::AddAssign;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureId(u64);

impl TextureId {
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl AddAssign<u64> for TextureId {
    fn add_assign(&mut self, rhs: u64) {
        self.0 = self.0.saturating_add(rhs);
    }
}

impl std::fmt::Display for TextureId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "TextureId({})", self.0)
    }
}

/// Where a rasterized tile's pixels live. GPU tiles are referenced by an opaque id only their
/// producing backend can resolve, which is what keeps this crate free of any dependency on `wgpu`.
#[derive(Debug, Clone)]
pub enum TilePixels {
    /// `Bytes` so tiles clone/slice into BakedTile / CachedTile without copying pixels. Byte order
    /// is given by the owning [`Texture`]'s `format`.
    Cpu(bytes::Bytes),
    /// Backend-owned GPU texture. Meaningful only to the backend that produced it.
    Gpu(GpuTile),
}

/// A GPU-resident tile's texture, by the opaque id its backend gave it.
///
/// Clones share one texture. When the last one is dropped -- the engine has re-rasterized the
/// tile, navigated away, or closed the tab -- the backend's release callback frees it. Without
/// that, a backend can only keep every texture it ever made.
#[derive(Clone)]
pub struct GpuTile(Arc<GpuTileInner>);

struct GpuTileInner {
    id: u64,
    release: Option<Box<dyn Fn(u64) + Send + Sync>>,
}

impl Drop for GpuTileInner {
    fn drop(&mut self) {
        if let Some(release) = &self.release {
            release(self.id);
        }
    }
}

impl GpuTile {
    /// A tile whose texture `release` frees once no clone is left.
    pub fn new(id: u64, release: impl Fn(u64) + Send + Sync + 'static) -> Self {
        Self(Arc::new(GpuTileInner {
            id,
            release: Some(Box::new(release)),
        }))
    }

    /// A tile with nothing to free, for tests and harnesses that never made a real texture.
    pub fn untracked(id: u64) -> Self {
        Self(Arc::new(GpuTileInner { id, release: None }))
    }

    /// The backend's id for the texture.
    pub fn id(&self) -> u64 {
        self.0.id
    }
}

impl std::fmt::Debug for GpuTile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuTile({})", self.0.id)
    }
}

/// A rasterized tile produced by a rasterizer, either CPU- or GPU-resident.
#[derive(Debug)]
pub struct Texture {
    pub id: TextureId,
    pub width: usize,
    pub height: usize,
    pub pixels: TilePixels,
    /// In-memory byte order of the pixels (CPU variant), set by the producing rasterizer.
    pub format: crate::render::backend::PixelFormat,
}

impl Texture {
    pub fn cpu_data(&self) -> Option<&bytes::Bytes> {
        match &self.pixels {
            TilePixels::Cpu(d) => Some(d),
            TilePixels::Gpu(_) => None,
        }
    }

    pub fn gpu_id(&self) -> Option<u64> {
        match &self.pixels {
            TilePixels::Gpu(tile) => Some(tile.id()),
            TilePixels::Cpu(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn gpu_tile_is_released_once_when_the_last_clone_drops() {
        let released = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&released);
        let tile = GpuTile::new(42, move |id| {
            assert_eq!(id, 42);
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let pixels = TilePixels::Gpu(tile.clone());
        drop(tile);
        assert_eq!(released.load(Ordering::SeqCst), 0, "a clone is still alive");
        drop(pixels);
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }
}
