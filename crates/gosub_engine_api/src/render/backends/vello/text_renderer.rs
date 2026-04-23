//! Text shaping and cached drawing for Vello using Parley.
//!
//! This module provides a small text pipeline that:
//! 1) resolves a font via your `FontManager`/`FontCache`,
//! 2) shapes text with Parley (`FontContext`/`LayoutContext`),
//! 3) caches positioned glyph runs keyed by [`TextKey`],
//! 4) draws the cached runs into a Vello [`Scene`].
//!
//! Caching avoids repeating the (relatively expensive) shaping step when you
//! draw the same text+font+size/wrap/alignment multiple times.

use crate::render::backends::vello::font_cache::FontCache;
use crate::render::backends::vello::font_manager::FontManager;
use parley::{Font, FontContext, LayoutContext};
#[cfg(not(feature = "parley_layout"))]
use skrifa::MetadataProvider;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use vello::kurbo::Affine;
use vello::peniko::{Brush, Color, Fill};
use vello::{Glyph, Scene};

/// Cache key for shaped text.
///
/// Two keys are considered equal if they match on
/// - `px`, `align`, `wrap`, and
/// - both `text` and `font_name` content (pointer equality *or* string equality).
///
/// Notes:
/// - `align` is present for future horizontal alignment logic (currently unused).
/// - `wrap` is the max line width in pixels (if `None` or `Some(0)`, lines are unbounded).
#[derive(Clone)]
pub struct TextKey {
    /// The text content to render.
    pub text: Arc<str>,
    /// The font family name to use.
    pub font_name: Arc<str>,
    /// Font size in pixels.
    pub font_size: u32,
    /// Optional max line width in pixels (if `None` or `Some(0)`, lines are unbounded).
    pub wrap: Option<u32>,
    /// Horizontal alignment: 0=left, 1=center, 2=right (currently unused).
    pub align: u8,
}

impl PartialEq for TextKey {
    fn eq(&self, o: &Self) -> bool {
        self.font_size == o.font_size
            && self.align == o.align
            && self.wrap == o.wrap
            && Arc::ptr_eq(&self.text, &o.text)
            && Arc::ptr_eq(&self.font_name, &o.font_name)
            || (self.text.as_ref() == o.text.as_ref() && self.font_name.as_ref() == o.font_name.as_ref())
    }
}

impl Eq for TextKey {}

impl Hash for TextKey {
    fn hash<H: Hasher>(&self, s: &mut H) {
        self.font_size.hash(s);
        self.align.hash(s);
        self.wrap.hash(s);
        self.text.as_ref().hash(s);
        self.font_name.as_ref().hash(s);
    }
}

/// A shaped glyph run ready to draw with Vello.
///
/// Each run carries:
/// - the resolved Vello [`Font`],
/// - the font size in pixels,
/// - the *absolute* glyph positions within the shaped block (y includes baseline/line offsets).
pub struct CachedRun {
    pub vello_font: Font,
    pub font_size: f32,
    pub glyphs: Arc<[Glyph]>,
}

/// Stateful text renderer that shapes text (via Parley) and draws it (via Vello),
/// with an internal cache keyed by [`TextKey`].
///
/// # Pipeline
/// - `shape()` resolves the font, builds a Parley layout, line-breaks it,
///   then converts positioned glyphs into Vello `Glyph`s with y that already
///   accounts for line height and baseline.
/// - `draw()` looks up/creates cached runs and submits them to the [`Scene`]
///   with a single affine translation for the target (x, y).
pub struct TextRenderer {
    font_cx: FontContext,
    layout_cx: LayoutContext<[u8; 4]>,
    cache: HashMap<TextKey, Arc<[CachedRun]>>,
}

impl TextRenderer {
    /// Create a fresh renderer with empty cache and shaping contexts.
    pub fn new() -> Self {
        Self {
            font_cx: FontContext::new(),
            layout_cx: LayoutContext::new(),
            cache: HashMap::new(),
        }
    }

    #[allow(unused)]
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Draw the given `key` at `(x, y)` with the RGBA color on the provided `scene`.
    ///
    /// If there is no cached shape for `key`, this will shape and cache it first.
    ///
    /// Coordinates:
    /// - `(x, y)` is the top-left origin for the shaped block (not the baseline).
    /// - Each cached run already encodes per-glyph baseline/line offsets.
    ///
    /// Performance:
    /// - Multiple calls with the same `key` reuse shaping work.
    /// - If you animate only the position/color, reuse the same `key`.
    pub fn draw(
        &mut self,
        fm: &mut FontManager,
        fc: &mut FontCache,
        scene: &mut Scene,
        key: &TextKey,
        x: f32,
        y: f32,
        rgba: [f32; 4],
    ) {
        let runs = if let Some(r) = self.cache.get(key) {
            r.clone()
        } else {
            let shaped = self.shape(fm, fc, key);
            self.cache.insert(key.clone(), shaped.clone());
            shaped
        };

        let scale = 1.0;
        let transform = Affine::translate((scale * x as f64, scale * y as f64));
        let brush = Brush::Solid(Color::new(rgba));

        for r in runs.iter() {
            scene
                .draw_glyphs(&r.vello_font)
                .font_size(r.font_size * scale as f32)
                .transform(transform)
                .brush(&brush)
                .brush_alpha(1.0)
                .draw(Fill::NonZero, r.glyphs.iter().copied());
        }
    }

    /// Shape `key.text` using Parley and return cached runs with absolute glyph positions.
    ///
    /// - Font resolution goes through the `FontManager`/`FontCache`.
    /// - Line breaking:
    ///   - If `wrap = Some(w) && w > 0`, lines are wrapped to `w` pixels.
    ///   - Otherwise, lines are unbounded (`INFINITY`).
    /// - Vertical metrics:
    ///   - For each line we compute:
    ///     - `lm = line.metrics()`
    ///     - `baseline = lm.ascent`
    ///     - add glyph `g.y`, and the run offset, onto `pen_y + baseline`
    ///   - `pen_y` accumulates `lm.line_height` per line.
    ///
    /// Alignment:
    /// - `align` is recorded in the key but currently not applied to advance/x positioning.
    ///   When adding alignment, adjust each line’s glyph x by the rag width delta.
    fn shape(&mut self, fm: &mut FontManager, fc: &mut FontCache, key: &TextKey) -> Arc<[CachedRun]> {
        // Resolve font
        let (vello_font, resolved_name) = match fc.fetch(&key.font_name) {
            Some(f) => (f.0.clone(), f.1),
            None => {
                let (vf, rn) = fm
                    .resolve_ui_font(Some(&key.font_name), fontique::Attributes::default())
                    .expect("resolve font");
                fc.insert(&key.font_name, rn.as_str(), vf.clone());
                (vf, rn)
            }
        };

        #[cfg(not(feature = "parley_layout"))]
        {
            let font_ref = to_font_ref(&vello_font).unwrap();
            let axes = font_ref.axes();
            let font_size = skrifa::instance::Size::new(key.font_size as f32);
            let var_loc = axes.location(std::iter::empty::<(&str, f32)>());
            let charmap = font_ref.charmap();
            let metrics = font_ref.metrics(font_size, &var_loc);
            let line_height = metrics.ascent - metrics.descent + metrics.leading;
            let glyph_metrics = font_ref.glyph_metrics(font_size, &var_loc);

            let mut pen_x = 0f32;
            let mut pen_y = 0f32;

            let glyphs = key
                .text
                .chars()
                .filter_map(|ch| {
                    if ch == '\n' {
                        pen_y += line_height;
                        pen_x = 0.0;
                        return None;
                    }
                    let gid = charmap.map(ch).unwrap_or_default();
                    let advance = glyph_metrics.advance_width(gid).unwrap_or_default();
                    let x = pen_x;
                    pen_x += advance;
                    Some(Glyph {
                        id: gid.to_u32(),
                        x,
                        y: pen_y,
                    })
                })
                .collect::<Arc<[_]>>();

            let mut out: Vec<CachedRun> = Vec::new();
            out.push(CachedRun {
                vello_font: vello_font.clone(),
                font_size: key.font_size as f32,
                glyphs: glyphs.into(),
            });

            out.into()
        }

        #[cfg(feature = "parley_layout")]
        {
            // Build layout
            let mut builder = self
                .layout_cx
                .ranged_builder(&mut self.font_cx, key.text.as_ref(), 1.0, true);
            builder.push_default(parley::style::StyleProperty::FontSize(key.font_size as f32));
            builder.push_default(parley::style::StyleProperty::FontStack(
                parley::style::FontStack::Single(parley::style::FontFamily::Named(resolved_name.into())),
            ));
            let mut layout = builder.build(key.text.as_ref());

            match key.wrap {
                Some(w) if w > 0 => {
                    let max_width = w as f32;
                    layout.break_all_lines(Some(max_width));
                }
                _ => {
                    let max_width = f32::INFINITY;
                    layout.break_all_lines(Some(max_width));
                }
            }

            let mut pen_y = 0.0f32;
            let mut out: Vec<CachedRun> = Vec::new();
            for line in layout.lines() {
                let lm = line.metrics();
                let baseline = lm.ascent as f32;

                for item in line.items() {
                    if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                        let ro = run.offset();

                        let glyphs: Vec<Glyph> = run
                            .positioned_glyphs()
                            .map(|g| Glyph {
                                id: g.id as u32,
                                x: g.x.round(),
                                y: (pen_y + baseline + ro + g.y).round(),
                            })
                            .collect();

                        out.push(CachedRun {
                            vello_font: vello_font.clone(),
                            font_size: key.font_size as f32,
                            glyphs: glyphs.into(),
                        });
                    }
                }

                pen_y += lm.line_height;
            }
            out.into()
        }
    }
}

#[cfg(not(feature = "parley_layout"))]
fn to_font_ref(font: &Font) -> Option<skrifa::raw::FontRef<'_>> {
    use skrifa::raw::FileRef;
    let file_ref = FileRef::new(font.data.as_ref()).ok()?;
    match file_ref {
        FileRef::Font(font) => Some(font),
        FileRef::Collection(collection) => collection.get(font.index).ok(),
    }
}
