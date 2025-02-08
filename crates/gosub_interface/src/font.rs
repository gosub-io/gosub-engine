use std::fmt::{Debug, Formatter};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct FontBlob {
    pub data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    pub index: u32,
}

impl FontBlob {
    pub fn new(data: Arc<dyn AsRef<[u8]> + Send + Sync>, index: u32) -> Self {
        Self { data, index }
    }

    pub fn data(&self) -> &Arc<dyn AsRef<[u8]> + Send + Sync> {
        &self.data
    }

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

pub trait HasFontManager: Sized + Debug {
    type FontManager: FontManager;
}

pub trait FontInfo: Sized + Clone + Debug + Send {
    fn family(&self) -> &str;
    fn style(&self) -> FontStyle;
    fn weight(&self) -> i32;
    fn stretch(&self) -> f32;
    fn monospaced(&self) -> bool;
    fn path(&self) -> Option<PathBuf>;
    fn index(&self) -> Option<i32>;

    fn new(family: &str) -> Result<Self, FontError>;
    fn with_family(&self, family: &str) -> Self;
    fn with_style(&self, style: FontStyle) -> Self;
    fn with_weight(&self, weight: i32) -> Self;
    fn with_stretch(&self, stretch: f32) -> Self;
    fn with_monospaced(&self, monospaced: bool) -> Self;
    fn with_path(&self, path: PathBuf, index: Option<i32>) -> Self;

    /// Converts this font info to a font description usable by Pango
    fn to_description(&self, size: f32) -> String;
}

pub trait FontManager: Sized + 'static {
    type FontInfo: FontInfo;

    fn instance() -> Arc<RwLock<Self>>;
    fn find_font(&self, families: &[&str], style: FontStyle) -> Option<Self::FontInfo>;
}
