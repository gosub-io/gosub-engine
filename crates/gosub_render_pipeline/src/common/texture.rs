use std::ops::AddAssign;

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

/// Where a rasterized tile's pixels live. The render pipeline is backend-agnostic and stays
/// free of any GPU API: a GPU-resident tile is referenced by an **opaque id** that only the
/// backend which created it can resolve (via its own texture store). This is what lets one tile
/// pipeline serve both CPU compositing backends (Cairo/Skia → `Cpu`) and GPU backends
/// (Vello/other wgpu → `Gpu`) without the core depending on `wgpu`.
#[derive(Debug, Clone)]
pub enum TilePixels {
    /// CPU pixel buffer, stored as `Bytes` so it can be cloned/shared (cheap refcount bump) and
    /// sliced zero-copy into BakedTile / CachedTile without any pixel buffer copies. Byte order is
    /// given by the owning [`Texture`]'s `format`.
    Cpu(bytes::Bytes),
    /// Opaque, backend-owned GPU texture id. Meaningful only to the backend that produced it.
    Gpu(u64),
}

/// A rasterized tile produced by a rasterizer, either CPU- or GPU-resident.
#[derive(Debug)]
pub struct Texture {
    pub id: TextureId,
    pub width: usize,
    pub height: usize,
    pub pixels: TilePixels,
    /// In-memory byte order of the pixels (for the CPU variant), set by the producing rasterizer.
    pub format: crate::render::backend::PixelFormat,
}

impl Texture {
    /// CPU pixel buffer, or `None` for a GPU-resident tile.
    pub fn cpu_data(&self) -> Option<&bytes::Bytes> {
        match &self.pixels {
            TilePixels::Cpu(d) => Some(d),
            TilePixels::Gpu(_) => None,
        }
    }

    /// Opaque GPU texture id, or `None` for a CPU tile.
    pub fn gpu_id(&self) -> Option<u64> {
        match &self.pixels {
            TilePixels::Gpu(id) => Some(*id),
            TilePixels::Cpu(_) => None,
        }
    }
}
