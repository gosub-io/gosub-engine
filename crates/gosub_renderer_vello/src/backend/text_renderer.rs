//! Text shaping and cached drawing for Vello using Parley.
//!
//! Shaped glyph runs are cached by [`TextKey`] so redrawing the same text+font+size/wrap/alignment
//! skips the (relatively expensive) shaping step.

use crate::backend::font_cache::FontCache;
use crate::backend::font_manager::FontManager;
#[cfg(not(feature = "parley_layout"))]
use parley::FontData as Font;
#[cfg(feature = "parley_layout")]
use parley::{FontData as Font, LayoutContext};
// FontContext is always in the draw/shape signatures so that the caller
// (VelloBackend) never needs cfg guards at the call site.
use parley::FontContext;
#[cfg(not(feature = "parley_layout"))]
use skrifa::MetadataProvider;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use vello::kurbo::Affine;
use vello::peniko::{Brush, Color, Fill};
use vello::{Glyph, Scene};

/// Cache key for shaped text.
#[derive(Clone)]
pub struct TextKey {
    pub text: Arc<str>,
    pub font_name: Arc<str>,
    /// Font size in pixels.
    pub font_size: u32,
    /// Max line width in pixels; `None` or `Some(0)` means unbounded.
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

/// A shaped glyph run ready to draw. Glyph positions are absolute within the shaped block —
/// `y` already includes baseline and line offsets.
pub struct CachedRun {
    pub vello_font: Font,
    pub font_size: f32,
    pub glyphs: Arc<[Glyph]>,
}

/// Shapes text via Parley and draws it via Vello, caching shaped runs by [`TextKey`].
///
/// The `FontContext` is injected by the caller rather than owned here, so all rendering components
/// share one font collection.
pub struct TextRenderer {
    cache: HashMap<TextKey, Arc<[CachedRun]>>,
}

impl TextRenderer {
    pub fn new() -> Self {
        Self { cache: HashMap::new() }
    }

    /// Draw `key` with `(x, y)` as the top-left of the shaped block (not the baseline); shapes and
    /// caches first if needed. Reusing a `key` across position/color changes reuses the shaping.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        fm: &mut FontManager,
        fc: &mut FontCache,
        font_cx: &mut FontContext,
        scene: &mut Scene,
        key: &TextKey,
        x: f32,
        y: f32,
        rgba: [f32; 4],
    ) {
        let runs = if let Some(r) = self.cache.get(key) {
            r.clone()
        } else {
            let shaped = self.shape(fm, fc, font_cx, key);
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

    /// Shape `key.text` into runs with absolute glyph positions (`y` includes baseline + line
    /// offsets). `key.align` is recorded but not yet applied to x positioning.
    ///
    /// The `parley_layout` feature selects the Parley shaping path; without it the skrifa path maps
    /// codepoints directly and ignores `font_cx`.
    #[cfg_attr(not(feature = "parley_layout"), allow(unused_variables))]
    fn shape(
        &mut self,
        fm: &mut FontManager,
        fc: &mut FontCache,
        font_cx: &mut FontContext,
        key: &TextKey,
    ) -> Arc<[CachedRun]> {
        let (vello_font, _resolved_name) = match fc.fetch(&key.font_name) {
            Some(f) => (f.0.clone(), f.1),
            None => match fm.resolve_ui_font(Some(&key.font_name), fontique::Attributes::default()) {
                Ok((vf, rn)) => {
                    fc.insert(&key.font_name, rn.as_str(), vf.clone());
                    (vf, rn)
                }
                Err(e) => {
                    // No font, no glyphs: drop this text run but keep rendering the frame.
                    log::warn!("Failed to resolve font '{}': {e}", key.font_name);
                    return Arc::from(Vec::new());
                }
            },
        };

        #[cfg(not(feature = "parley_layout"))]
        {
            let Some(font_ref) = to_font_ref(&vello_font) else {
                log::warn!("Could not read font data for '{}'; dropping text run", key.font_name);
                return Arc::from(Vec::new());
            };
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

            let out: Vec<CachedRun> = vec![CachedRun {
                vello_font: vello_font.clone(),
                font_size: key.font_size as f32,
                glyphs,
            }];

            out.into()
        }

        #[cfg(feature = "parley_layout")]
        {
            // A fresh LayoutContext is cheap (it is pure scratch space).
            // FontContext is the expensive shared state - it is injected by the caller.
            let mut layout_cx: LayoutContext<[u8; 4]> = LayoutContext::new();

            let mut builder = layout_cx.ranged_builder(font_cx, key.text.as_ref(), 1.0, true);
            builder.push_default(parley::style::StyleProperty::FontSize(key.font_size as f32));
            builder.push_default(parley::style::StyleProperty::FontFamily(
                parley::style::FontFamily::Source(_resolved_name.into()),
            ));
            let mut layout = builder.build(key.text.as_ref());

            let max_width = match key.wrap {
                Some(w) if w > 0 => w as f32,
                _ => f32::INFINITY,
            };
            layout.break_all_lines(Some(max_width));

            let mut pen_y = 0.0f32;
            let mut out: Vec<CachedRun> = Vec::new();
            for line in layout.lines() {
                let lm = line.metrics();
                let baseline = lm.ascent;

                for item in line.items() {
                    if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                        let ro = run.offset();

                        let glyphs: Vec<Glyph> = run
                            .positioned_glyphs()
                            .map(|g| Glyph {
                                id: g.id,
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
