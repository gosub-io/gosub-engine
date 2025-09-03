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

impl AddAssign<i32> for TextureId {
    fn add_assign(&mut self, rhs: i32) {
        self.0 += rhs as u64;
    }
}

impl std::fmt::Display for TextureId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "TextureId({})", self.0)
    }
}

// Texture is a simple structure that holds the texture data. It's raw data, width, height and an id.
// Note that we do not specify what the data contains. It could be a specific image format from the
// used painting backend (like ImageSurface data for cairo)
#[derive(Debug)]
pub struct Texture {
    pub id: TextureId,
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}
