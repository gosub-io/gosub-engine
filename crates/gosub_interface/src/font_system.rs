use parking_lot::Mutex;
use std::any::Any;
use std::sync::Arc;

use crate::font::{FontBlob, FontError, FontStyle};

// Value types

/// CSS font-weight (100–900). Common constants provided for convenience.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct FontWeight(pub u16);

impl FontWeight {
    pub const THIN: Self = Self(100);
    pub const EXTRA_LIGHT: Self = Self(200);
    pub const LIGHT: Self = Self(300);
    pub const NORMAL: Self = Self(400);
    pub const MEDIUM: Self = Self(500);
    pub const SEMI_BOLD: Self = Self(600);
    pub const BOLD: Self = Self(700);
    pub const EXTRA_BOLD: Self = Self(800);
    pub const BLACK: Self = Self(900);
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// CSS font-stretch as a multiplier (1.0 = normal, 0.5 = condensed, 2.0 = expanded).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct FontStretch(pub f32);

impl FontStretch {
    pub const ULTRA_CONDENSED: Self = Self(0.5);
    pub const CONDENSED: Self = Self(0.75);
    pub const NORMAL: Self = Self(1.0);
    pub const EXPANDED: Self = Self(1.25);
    pub const ULTRA_EXPANDED: Self = Self(2.0);
}

impl Default for FontStretch {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// A CSS font-family query with full property set.
///
/// `families` is a priority-ordered slice of family names, exactly as they appear
/// in the CSS `font-family` property (e.g. `["Helvetica Neue", "Arial", "sans-serif"]`).
#[derive(Debug, Clone)]
pub struct FontQuery<'a> {
    pub families: &'a [&'a str],
    pub style: FontStyle,
    pub weight: FontWeight,
    pub stretch: FontStretch,
}

impl<'a> FontQuery<'a> {
    pub fn new(families: &'a [&'a str]) -> Self {
        Self {
            families,
            style: FontStyle::Normal,
            weight: FontWeight::NORMAL,
            stretch: FontStretch::NORMAL,
        }
    }
}

/// A concrete font that has been resolved from a `FontQuery`.
///
/// Carries the raw font bytes so both the layout engine and the renderer can
/// use the same data without going back to the font system.
#[derive(Debug, Clone)]
pub struct ResolvedFont {
    /// The family name that was actually selected (may differ from what was requested
    /// if a fallback was used).
    pub family: String,
    pub style: FontStyle,
    pub weight: FontWeight,
    pub stretch: FontStretch,
    /// Raw font bytes + collection index.
    pub blob: FontBlob,
}

// Shaping output

/// A single positioned glyph.
///
/// `x` and `y` are in pixels, with `y` already including the baseline and any
/// line offsets — (0, 0) is the top-left of the shaped block, not the baseline.
#[derive(Debug, Clone, Copy)]
pub struct ShapedGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// A contiguous run of glyphs rendered with the same font and size.
///
/// A single call to `FontSystem::shape` may return multiple runs when font
/// fallback kicks in mid-string (e.g. an emoji in a Latin text run).
#[derive(Debug, Clone)]
pub struct ShapedRun {
    pub font: ResolvedFont,
    pub font_size: f32,
    pub glyphs: Vec<ShapedGlyph>,
}

/// The complete result of shaping a string: positioned glyph runs plus metrics.
#[derive(Debug, Clone)]
pub struct ShapedText {
    /// One entry per font run. May span multiple lines.
    pub runs: Vec<ShapedRun>,
    /// Bounding box of all runs.
    pub width: f32,
    pub height: f32,
    /// Dominant line height (useful for cursor positioning and decoration).
    pub line_height: f32,
    /// Baseline of the first line, measured from the top of the bounding box.
    pub ascent: f32,
}

impl ShapedText {
    pub fn empty() -> Self {
        Self {
            runs: Vec::new(),
            width: 0.0,
            height: 0.0,
            line_height: 0.0,
            ascent: 0.0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

// Text style for measurement

/// CSS-resolved text style passed to [`FontSystem::measure`].
///
/// Carries everything an engine needs to lay out a run of text: the family (the implementation
/// appends its own generic/bundled fallback), size, the font selectors, an optional absolute line
/// height, an optional wrap width, and the device-pixel scale.
#[derive(Debug, Clone)]
pub struct TextStyle {
    /// Primary CSS family name (implementations append a generic/bundled fallback).
    pub family: String,
    /// Font size in CSS pixels.
    pub size: f32,
    pub weight: FontWeight,
    pub style: FontStyle,
    pub stretch: FontStretch,
    /// `Some(px)` forces an absolute line-box height; `None` = the font's natural height.
    pub line_height: Option<f32>,
    /// `Some(px)` soft-wraps at that width; `None` = a single unbroken line.
    pub max_width: Option<f32>,
    /// Device-pixel scale (DPI). `1.0` = CSS pixels.
    pub display_scale: f32,
}

impl TextStyle {
    /// A style for `family` at `size` px, default selectors, scale 1.0, no wrap.
    pub fn new(family: impl Into<String>, size: f32) -> Self {
        Self {
            family: family.into(),
            size,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            stretch: FontStretch::NORMAL,
            line_height: None,
            max_width: None,
            display_scale: 1.0,
        }
    }
}

// Core trait

/// A swappable font system — the entire surface the engine and layouter need.
///
/// It registers fonts and **measures** text. Measuring goes through whichever font system the
/// engine was configured with, so layout boxes are sized by the very engine that will draw the
/// text (Parley, Pango, Skia, …) — measurement and drawing can't disagree.
///
/// Engine-native *shaping* and *drawing* are deliberately **not** on this trait: the shaped
/// representation and the draw target are backend-specific. A render backend recovers its concrete
/// font system via [`FontSystem::as_any_mut`] and calls that type's own (inherent) shaping/draw
/// methods.
///
/// # Threading
/// `Send + Sync` so it can live behind `Arc<Mutex<dyn FontSystem>>`, shared between the layouter
/// and the renderer.
pub trait FontSystem: Send + Sync + 'static {
    /// Register a font from raw bytes (`@font-face` web fonts, bundled fallbacks).
    ///
    /// `family_override` assigns a logical name CSS can reference; `None` uses the font's own name.
    fn register_font(&mut self, data: Vec<u8>, family_override: Option<&str>) -> Result<(), FontError>;

    /// Measure the bounding box of `text` laid out in `style`, in CSS pixels.
    fn measure(&mut self, text: &str, style: &TextStyle) -> (f32, f32);

    /// Recover the concrete font system so a render backend can call its native shaping/draw path.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

// Config integration

/// Marker trait: a config type `C` that carries a `FontSystem`.
///
/// Implement this on your top-level `Config` struct, then pass
/// `Arc<Mutex<dyn FontSystem>>` into both the layout engine and the renderer.
///
/// ```ignore
/// impl HasFontSystem for MyConfig {
///     fn font_system(&self) -> Arc<Mutex<dyn FontSystem>> {
///         Arc::clone(&self.font_system)
///     }
/// }
/// ```
pub trait HasFontSystem {
    fn font_system(&self) -> Arc<Mutex<dyn FontSystem>>;
}
