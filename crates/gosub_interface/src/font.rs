use std::fmt::{Debug, Formatter};
use std::sync::Arc;

#[derive(Clone)]
pub struct FontBlob {
    pub data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    pub index: u32,
}

impl FontBlob {
    pub fn new(data: Arc<dyn AsRef<[u8]> + Send + Sync>, index: u32) -> Self {
        Self { data, index }
    }

    #[must_use]
    pub fn data(&self) -> &Arc<dyn AsRef<[u8]> + Send + Sync> {
        &self.data
    }

    #[must_use]
    pub fn as_u8(&self) -> &[u8] {
        self.data.as_ref().as_ref()
    }
}

impl Debug for FontBlob {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Font").finish()
    }
}

#[derive(Debug)]
pub enum FontError {
    FontNotFound(String),       // Font not found for a family
    InvalidFont(String),        // Font is invalid or corrupted
    UnsupportedFeature(String), // Unsupported features
}

#[derive(Clone, Debug, Copy, PartialEq)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

impl std::fmt::Display for FontStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            FontStyle::Normal => write!(f, "Normal"),
            FontStyle::Italic => write!(f, "Italic"),
            FontStyle::Oblique => write!(f, "Oblique"),
        }
    }
}
