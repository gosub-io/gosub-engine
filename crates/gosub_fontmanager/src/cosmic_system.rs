//! A second [`FontSystem`] implementation, backed by **cosmic-text** (fontdb discovery +
//! rustybuzz shaping + swash). It implements exactly the same trait as [`crate::ParleyFontSystem`],
//! demonstrating that the font abstraction is engine-agnostic — the layouter can measure with it,
//! and a backend that paints [`ShapedText`] glyph runs can render with it.
//!
//! Note: cosmic-text doesn't expose the underlying shared font bytes, so `blob_for` copies them
//! when filling a [`FontBlob`] (cached upstream by whoever holds the `ResolvedFont`).

use cosmic_text::{
    fontdb, Align, Attrs, Buffer, Family, FontSystem as CosmicTextFontSystem, Metrics, Shaping, Stretch, Style,
    Weight,
};
use cow_utils::CowUtils;
use gosub_interface::font::{FontBlob, FontError, FontStyle};
use gosub_interface::font_system::{
    FontQuery, FontStretch, FontSystem, ResolvedFont, RunMetrics, ShapedGlyph, ShapedRun, ShapedText, TextAlign,
    TextStyle,
};
use std::any::Any;
use std::sync::Arc;

/// A [`FontSystem`] backed by cosmic-text.
pub struct CosmicFontSystem {
    inner: CosmicTextFontSystem,
}

impl std::fmt::Debug for CosmicFontSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CosmicFontSystem").finish_non_exhaustive()
    }
}

impl Default for CosmicFontSystem {
    fn default() -> Self {
        Self::new()
    }
}

/// A run of shaped glyphs that all share one font. Collected while the cosmic-text `buffer`
/// is borrowed, so the per-run font-blob lookup (which borrows `self.inner`) can run afterward
/// without overlapping borrows.
struct RawRun {
    id: fontdb::ID,
    weight: Weight,
    x: f32,
    baseline: f32,
    width: f32,
    glyphs: Vec<ShapedGlyph>,
}

/// Decoration metrics estimated from the font size.
///
/// cosmic-text doesn't surface the font's own underline/strikeout tables, so use the common
/// conventions (underline ~1/10 em below the baseline, strikeout ~1/4 em above, both ~1/14 em
/// thick) until a swash-based lookup replaces this.
fn heuristic_metrics(size: f32) -> RunMetrics {
    RunMetrics {
        underline_offset: size * 0.1,
        underline_size: size / 14.0,
        strikethrough_offset: -size * 0.25,
        strikethrough_size: size / 14.0,
    }
}

impl CosmicFontSystem {
    /// Create a font system with system fonts loaded and Roboto registered as a bundled fallback.
    pub fn new() -> Self {
        let mut inner = CosmicTextFontSystem::new();
        // Load the bundled Roboto without copying: the static bytes already implement
        // `AsRef<[u8]>`, so hand fontdb a shared reference instead of an owned `Vec`.
        inner
            .db_mut()
            .load_font_source(fontdb::Source::Binary(Arc::new(gosub_shared::ROBOTO_FONT)));
        Self { inner }
    }

    /// Build and shape a cosmic-text buffer for `text` in the given style.
    fn shaped_buffer(&mut self, text: &str, style: &TextStyle) -> Buffer {
        let metrics = Metrics::new(style.size, style.line_height.unwrap_or(style.size * 1.2));
        let mut buffer = Buffer::new(&mut self.inner, metrics);
        buffer.set_size(style.max_width, None);
        let attrs = Attrs::new()
            .family(css_family(&style.family))
            .weight(Weight(style.weight.0))
            .style(to_style(style.style))
            .stretch(to_stretch(style.stretch));
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        let align = match style.align {
            TextAlign::Start => None, // natural per-direction default
            TextAlign::Center => Some(Align::Center),
            TextAlign::End => Some(Align::End),
            TextAlign::Justify => Some(Align::Justified),
        };
        if align.is_some() {
            for line in buffer.lines.iter_mut() {
                line.set_align(align);
            }
        }
        buffer.shape_until_scroll(&mut self.inner, false);
        buffer
    }

    /// Raw font bytes for a resolved face, as a [`FontBlob`].
    ///
    /// cosmic-text doesn't expose the underlying shared `Arc<[u8]>`, so this copies the file
    /// bytes and assumes face index 0 (correct for single-face files; `.ttc` collections would
    /// need the real index). Only used to fill `FontBlob`, which a cosmic draw path doesn't yet
    /// consume — so the copy is harmless for now.
    fn blob_for(&mut self, id: fontdb::ID, weight: Weight) -> Option<FontBlob> {
        let font = self.inner.get_font(id, weight)?;
        Some(FontBlob::new(Arc::new(font.data().to_vec()), 0))
    }
}

impl FontSystem for CosmicFontSystem {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn register_font(&mut self, data: Vec<u8>, _family_override: Option<&str>) -> Result<(), FontError> {
        // fontdb derives the family name from the font's own `name` table; overrides unsupported.
        self.inner.db_mut().load_font_data(data);
        Ok(())
    }

    fn measure(&mut self, text: &str, style: &TextStyle) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        let buffer = self.shaped_buffer(text, style);
        let mut width = 0.0f32;
        let mut height = 0.0f32;
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            height += run.line_height;
        }
        (width, height)
    }

    /// Resolve a CSS font query to a concrete font via fontdb.
    fn resolve(&mut self, query: &FontQuery<'_>) -> Result<ResolvedFont, FontError> {
        let mut families: Vec<Family> = query.families.iter().map(|f| css_family(f)).collect();
        // Bundled last-resort fallback so resolution always succeeds even with no system fonts
        // (e.g. headless/CI) — Roboto is registered in `new()`.
        families.push(Family::Name("Roboto"));
        let weight = Weight(query.weight.0);
        let fq = fontdb::Query {
            families: &families,
            weight,
            stretch: to_stretch(query.stretch),
            style: to_style(query.style),
        };

        let id = self
            .inner
            .db_mut()
            .query(&fq)
            .ok_or_else(|| FontError::FontNotFound(query.families.join(", ")))?;
        let blob = self
            .blob_for(id, weight)
            .ok_or_else(|| FontError::FontNotFound(query.families.join(", ")))?;

        Ok(ResolvedFont {
            family: query.families.first().copied().unwrap_or("sans-serif").to_string(),
            style: query.style,
            weight: query.weight,
            stretch: query.stretch,
            blob,
        })
    }

    /// Shape `text` into positioned glyph runs.
    fn shape(&mut self, text: &str, style: &TextStyle) -> ShapedText {
        if text.is_empty() {
            return ShapedText::empty();
        }

        let buffer = self.shaped_buffer(text, style);

        // Collect owned run data first (borrows `buffer`), then look up font blobs afterwards
        // (borrows `self.inner`) so the two borrows don't overlap.
        let mut raw: Vec<RawRun> = Vec::new();
        let mut width = 0.0f32;
        let mut height = 0.0f32;
        let mut ascent = 0.0f32;
        let mut line_height_out = 0.0f32;
        let mut first = true;

        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            line_height_out = run.line_height;
            if first {
                ascent = run.line_y; // baseline of the first line, from the top
                first = false;
            }

            // Split each line into runs of a single font (cosmic substitutes per-glyph fallback).
            let mut i = 0;
            while i < run.glyphs.len() {
                let fid = run.glyphs[i].font_id;
                let fw = run.glyphs[i].font_weight;
                let run_x = run.glyphs[i].x;
                let mut run_width = 0.0f32;
                let mut glyphs = Vec::new();
                while i < run.glyphs.len() && run.glyphs[i].font_id == fid {
                    let g = &run.glyphs[i];
                    glyphs.push(ShapedGlyph {
                        id: g.glyph_id as u32,
                        x: g.x,
                        y: run.line_y + g.y,
                    });
                    run_width = (g.x + g.w) - run_x;
                    i += 1;
                }
                raw.push(RawRun {
                    id: fid,
                    weight: fw,
                    x: run_x,
                    baseline: run.line_y,
                    width: run_width,
                    glyphs,
                });
            }

            height += run.line_height;
        }
        drop(buffer);

        let runs = raw
            .into_iter()
            .filter_map(|r| {
                let blob = self.blob_for(r.id, r.weight)?;
                Some(ShapedRun {
                    font: ResolvedFont {
                        family: style.family.clone(),
                        style: style.style,
                        weight: style.weight,
                        stretch: style.stretch,
                        blob,
                    },
                    font_size: style.size,
                    x: r.x,
                    baseline: r.baseline,
                    width: r.width,
                    metrics: heuristic_metrics(style.size),
                    glyphs: r.glyphs,
                })
            })
            .collect();

        ShapedText {
            runs,
            width,
            height,
            line_height: line_height_out,
            ascent,
        }
    }
}

// ── conversions: our neutral font-query types → cosmic-text/fontdb types ──

fn css_family(name: &str) -> Family<'_> {
    match name.cow_to_lowercase().as_ref() {
        "sans-serif" => Family::SansSerif,
        "serif" => Family::Serif,
        "monospace" | "monospaced" => Family::Monospace,
        "cursive" => Family::Cursive,
        "fantasy" => Family::Fantasy,
        _ => Family::Name(name),
    }
}

fn to_style(s: FontStyle) -> Style {
    match s {
        FontStyle::Normal => Style::Normal,
        FontStyle::Italic => Style::Italic,
        FontStyle::Oblique => Style::Oblique,
    }
}

fn to_stretch(s: FontStretch) -> Stretch {
    let r = s.0;
    if r <= 0.5625 {
        Stretch::UltraCondensed
    } else if r <= 0.6875 {
        Stretch::ExtraCondensed
    } else if r <= 0.8125 {
        Stretch::Condensed
    } else if r <= 0.9375 {
        Stretch::SemiCondensed
    } else if r < 1.0625 {
        Stretch::Normal
    } else if r < 1.1875 {
        Stretch::SemiExpanded
    } else if r < 1.375 {
        Stretch::Expanded
    } else if r < 1.75 {
        Stretch::ExtraExpanded
    } else {
        Stretch::UltraExpanded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_interface::font_system::FontQuery;

    #[test]
    fn resolves_measures_and_shapes() {
        let mut fs = CosmicFontSystem::new();
        let query = FontQuery::new(&["sans-serif"]);
        let resolved = fs.resolve(&query).expect("sans-serif should resolve (Roboto fallback)");

        assert!(!resolved.blob.as_u8().is_empty(), "resolved font must carry its bytes");

        let mut style = TextStyle::new("sans-serif", 16.0);
        style.line_height = Some(19.2);
        let (w, h) = fs.measure("Hello", &style);
        assert!(w > 0.0 && h > 0.0, "expected a non-zero measurement, got {w} x {h}");

        let shaped = fs.shape("Hello", &style);
        assert!(!shaped.runs.is_empty(), "expected at least one shaped run");
        let glyphs: usize = shaped.runs.iter().map(|r| r.glyphs.len()).sum();
        assert!(glyphs >= 5, "expected >= 5 glyphs for \"Hello\", got {glyphs}");
    }
}
