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

/// Raw pixel buffer produced by a rasterizer. Data is Arc-wrapped so it can be shared
/// zero-copy into BakedTile / CachedTile without any pixel buffer copies.
#[derive(Debug)]
pub struct Texture {
    pub id: TextureId,
    pub width: usize,
    pub height: usize,
    pub data: std::sync::Arc<Vec<u8>>,
    /// In-memory byte order of `data`, set by the rasterizer that produced it.
    pub format: crate::render::backend::PixelFormat,
}
