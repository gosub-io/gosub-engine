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

/// Decoration metrics for a shaped run, in pixels.
///
/// Offsets are measured from the run's baseline to the **top** of the stroke, positive
/// **downward** (so `underline_offset` is typically positive, `strikethrough_offset` typically
/// negative). A painter draws a decoration as a filled rect at
/// `(run.x, run.baseline + offset, run.width, size)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunMetrics {
    pub underline_offset: f32,
    pub underline_size: f32,
    pub strikethrough_offset: f32,
    pub strikethrough_size: f32,
}

/// A contiguous run of glyphs rendered with the same font and size.
///
/// A single call to `FontSystem::shape` may return multiple runs when font
/// fallback kicks in mid-string (e.g. an emoji in a Latin text run).
#[derive(Debug, Clone)]
pub struct ShapedRun {
    pub font: ResolvedFont,
    pub font_size: f32,
    /// Horizontal start of the run within the shaped block, px.
    pub x: f32,
    /// Baseline of the run's line, px from the top of the shaped block.
    pub baseline: f32,
    /// Advance width of the run, px.
    pub width: f32,
    /// Decoration metrics of the run's font, for underline/strikethrough painting.
    pub metrics: RunMetrics,
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

/// CSS `text-align`, applied during shaping within [`TextStyle::max_width`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

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
    /// Extra spacing between characters in px (CSS `letter-spacing`; 0 = `normal`). Affects the
    /// measured width, so it must match what the renderer draws.
    pub letter_spacing: f32,
    /// `Some(px)` soft-wraps at that width; `None` = a single unbroken line.
    pub max_width: Option<f32>,
    /// Alignment of the shaped lines within `max_width` (no-op when `max_width` is `None`).
    pub align: TextAlign,
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
            letter_spacing: 0.0,
            max_width: None,
            align: TextAlign::Start,
            display_scale: 1.0,
        }
    }
}

// Core trait

/// A swappable font system — the entire surface the engine and layouter need.
///
/// It registers fonts, **resolves** CSS font queries to concrete fonts (with their raw bytes),
/// **shapes** text into positioned glyph runs, and **measures** it. All of it goes through
/// whichever font system the engine was configured with, so layout boxes are sized by the very
/// engine whose glyphs will be drawn (Parley, Pango, Skia, cosmic-text, …) — measurement,
/// shaping, and drawing can't disagree.
///
/// Drawing itself is *not* on this trait: painting the [`ShapedText`] returned by
/// [`FontSystem::shape`] is the render backend's job (glyph IDs + a [`crate::font::FontBlob`]
/// are everything a rasterizer needs). Until every backend consumes `ShapedText`, backends with
/// an engine-native draw path recover their concrete font system via [`FontSystem::as_any_mut`];
/// that escape hatch is scheduled for removal.
///
/// # Threading
/// `Send + Sync` so it can live behind `Arc<Mutex<dyn FontSystem>>`, shared between the layouter
/// and the renderer.
pub trait FontSystem: Send + Sync + 'static {
    /// Register a font from raw bytes (`@font-face` web fonts, bundled fallbacks).
    ///
    /// `family_override` assigns a logical name CSS can reference; `None` uses the font's own name.
    fn register_font(&mut self, data: Vec<u8>, family_override: Option<&str>) -> Result<(), FontError>;

    /// Resolve a CSS font query to a concrete font, including its raw bytes.
    ///
    /// Walks `query.families` in priority order (generic keywords like `sans-serif` map to the
    /// engine's platform fallback) and returns the first matching face. The returned
    /// [`ResolvedFont::family`] is the family that was actually selected, which may differ from
    /// every requested name when the engine fell back.
    fn resolve(&mut self, query: &FontQuery<'_>) -> Result<ResolvedFont, FontError>;

    /// Shape `text` laid out in `style` into positioned glyph runs.
    ///
    /// Handles family resolution, line breaking (at `style.max_width`), and mid-string font
    /// fallback internally; each returned [`ShapedRun`] names the font that was *actually* used
    /// for its glyphs, so a rasterizer can draw the runs without consulting the font system
    /// again. Returns [`ShapedText::empty`] for empty input or when no font resolves.
    fn shape(&mut self, text: &str, style: &TextStyle) -> ShapedText;

    /// Measure the bounding box of `text` laid out in `style`, in CSS pixels.
    ///
    /// The default implementation shapes and reads the bounding box, guaranteeing measurement
    /// agrees with what [`FontSystem::shape`] produces; implementations may override with a
    /// cheaper path as long as they preserve that agreement.
    fn measure(&mut self, text: &str, style: &TextStyle) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        let shaped = self.shape(text, style);
        (shaped.width, shaped.height)
    }

    /// Recover the concrete font system so a render backend can call its native draw path.
    ///
    /// Transitional: goes away once every backend paints [`ShapedText`] instead.
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
