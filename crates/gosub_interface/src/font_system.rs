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

// Core trait

/// A swappable font back-end.
///
/// Implementations exist (or will exist) for:
/// - `ParleyFontSystem` — uses parley + fontique (default, supports Vello/Skia)
/// - `PangoFontSystem`  — wraps GTK/Pango (used by the Cairo renderer on Linux)
///
/// The system is intended to be shared between the layout engine (Taffy) and the
/// renderer so that shaping is done once and the resulting `ShapedText` is passed
/// through the pipeline rather than re-shaped at draw time.
///
/// # Threading
/// `FontSystem` requires `Send + Sync` so it can be wrapped in `Arc<Mutex<dyn FontSystem>>`.
/// Callers must hold the mutex across a full resolve+shape pair if they need
/// the results to be consistent.
pub trait FontSystem: Send + Sync + 'static {
    /// Required for downcasting to a concrete font system implementation.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Register a font from raw bytes.
    ///
    /// Used for `@font-face` web fonts and for bundled fallback fonts (e.g. Roboto).
    /// `family_override` lets callers assign a logical name that CSS can reference;
    /// if `None` the name is taken from the font's own `name` table.
    fn register_font(&mut self, data: Vec<u8>, family_override: Option<&str>) -> Result<(), FontError>;

    /// Resolve a CSS font query to a concrete `ResolvedFont`.
    ///
    /// Walks `query.families` in order and applies weight/style/stretch matching.
    /// Returns `Err(FontError::FontNotFound)` only if no family in the list (including
    /// generic families like `sans-serif`) could be resolved.
    fn resolve(&mut self, query: &FontQuery<'_>) -> Result<ResolvedFont, FontError>;

    /// Shape `text` with `font` at `size` pixels.
    ///
    /// `max_width`: if `Some(w)`, soft-wrap lines at `w` pixels. `None` = single line.
    ///
    /// Returns fully positioned glyphs. The returned `ShapedText` is cheaply cloneable
    /// (glyphs are usually stored in an `Arc<[ShapedGlyph]>` or `Vec`) so callers can
    /// cache it at the layout node level.
    fn shape(&mut self, text: &str, font: &ResolvedFont, size: f32, max_width: Option<f32>) -> ShapedText;

    /// Return only the bounding box of `text` without glyph positions.
    ///
    /// The default implementation delegates to `shape()`. Override this when the
    /// back-end has a cheaper measurement path (e.g. `pango::Layout::pixel_size()`).
    fn measure(&mut self, text: &str, font: &ResolvedFont, size: f32, max_width: Option<f32>) -> (f32, f32) {
        let shaped = self.shape(text, font, size, max_width);
        (shaped.width, shaped.height)
    }
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
