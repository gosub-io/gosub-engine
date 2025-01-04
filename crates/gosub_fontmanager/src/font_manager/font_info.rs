use std::path::PathBuf;

use gosub_interface::font::{FontError, FontInfo as TFontInfo, FontStyle};

#[derive(Clone, Debug)]
pub struct FontInfo {
    /// Family name of the font (e.g. "Arial")
    pub family: String,
    /// Style of the font
    pub style: FontStyle,
    /// Weight (400 normal, 700 bold)
    pub weight: i32,
    /// Stretch (1.0 normal, < 1.0 condensed)
    pub stretch: f32,
    /// Font is monospaced
    pub monospaced: bool,
    /// Path to the font file
    pub path: Option<PathBuf>,
    /// Index of the face in the font-file
    pub index: Option<i32>,
}

impl TFontInfo for FontInfo {
    fn family(&self) -> &str {
        self.family.as_str()
    }

    fn style(&self) -> FontStyle {
        self.style
    }

    fn weight(&self) -> i32 {
        self.weight
    }

    fn stretch(&self) -> f32 {
        self.stretch
    }

    fn monospaced(&self) -> bool {
        self.monospaced
    }

    fn path(&self) -> Option<PathBuf> {
        self.path.clone()
    }

    fn index(&self) -> Option<i32> {
        self.index
    }

    fn new(family: &str) -> Result<Self, FontError> {
        Ok(Self {
            family: family.to_string(),
            style: FontStyle::Normal,
            weight: 400,
            stretch: 1.0,
            monospaced: false,
            path: None,
            index: None,
        })
    }

    fn with_family(&self, family: &str) -> Self {
        Self {
            family: family.to_string(),
            style: self.style,
            weight: self.weight,
            stretch: self.stretch,
            monospaced: self.monospaced,
            path: self.path.clone(),
            index: self.index,
        }
    }

    fn with_style(&self, style: FontStyle) -> Self {
        Self {
            family: self.family.clone(),
            style,
            weight: self.weight,
            stretch: self.stretch,
            monospaced: self.monospaced,
            path: self.path.clone(),
            index: self.index,
        }
    }

    fn with_weight(&self, weight: i32) -> Self {
        Self {
            family: self.family.clone(),
            style: self.style,
            weight,
            stretch: self.stretch,
            monospaced: self.monospaced,
            path: self.path.clone(),
            index: self.index,
        }
    }

    fn with_stretch(&self, stretch: f32) -> Self {
        Self {
            family: self.family.clone(),
            style: self.style,
            weight: self.weight,
            stretch,
            monospaced: self.monospaced,
            path: self.path.clone(),
            index: self.index,
        }
    }

    fn with_monospaced(&self, monospaced: bool) -> Self {
        Self {
            family: self.family.clone(),
            style: self.style,
            weight: self.weight,
            stretch: self.stretch,
            monospaced,
            path: self.path.clone(),
            index: self.index,
        }
    }

    fn with_path(&self, path: PathBuf, index: Option<i32>) -> Self {
        Self {
            family: self.family.clone(),
            style: self.style,
            weight: self.weight,
            stretch: self.stretch,
            monospaced: self.monospaced,
            path: Some(path),
            index,
        }
    }

    fn to_description(&self, size: f32) -> String {
        format!("{} {} {}", self.family, self.style, size)
    }
}
