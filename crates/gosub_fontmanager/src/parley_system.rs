use cow_utils::CowUtils;
use gosub_interface::font::{FontBlob, FontError, FontStyle};
use gosub_interface::font_system::{
    FontQuery, FontStretch, FontSystem, FontWeight, ResolvedFont, RunMetrics, ShapedGlyph, ShapedRun, ShapedText,
    TextAlign, TextStyle,
};
use parley::fontique::{Attributes, FontWidth, GenericFamily, QueryFamily, QueryStatus, SourceCache};
use parley::style::{FontStyle as ParleyStyle, FontWeight as ParleyWeight};
use parley::{Alignment, AlignmentOptions, FontContext, LayoutContext, PositionedLayoutItem};
use std::any::Any;

/// A [`FontSystem`] implementation backed by Parley + Fontique.
///
/// Holds a single `FontContext` (fontique collection) and `LayoutContext` so that
/// all callers — the layout engine and every renderer — share the same font data and
/// produce consistent glyph metrics.
///
/// Construct once at application start, wrap in `Arc<Mutex<ParleyFontSystem>>`, and
/// pass the same `Arc` into both the Taffy layouter and the rendering backend.
pub struct ParleyFontSystem {
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,
    source_cache: SourceCache,
}

impl std::fmt::Debug for ParleyFontSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParleyFontSystem").finish_non_exhaustive()
    }
}

impl Default for ParleyFontSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl ParleyFontSystem {
    /// Create a new font system with system fonts loaded and Roboto registered as
    /// the built-in fallback.
    pub fn new() -> Self {
        let mut font_cx = FontContext::new();

        // Register Roboto as a bundled fallback so there is always something to
        // render with even on systems that have no fonts installed.
        font_cx
            .collection
            .register_fonts(gosub_shared::ROBOTO_FONT.to_vec().into(), None);

        Self {
            font_cx,
            layout_cx: LayoutContext::new(),
            source_cache: SourceCache::new_shared(),
        }
    }
}

impl ParleyFontSystem {
    /// Grants direct access to the underlying Parley font collection.
    ///
    /// Used by `TaffyLayouter` so that the same font collection is shared between
    /// the layout engine and rendering, ensuring consistent shaping.
    pub fn font_cx_mut(&mut self) -> &mut FontContext {
        &mut self.font_cx
    }
}

impl FontSystem for ParleyFontSystem {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn register_font(&mut self, data: Vec<u8>, _family_override: Option<&str>) -> Result<(), FontError> {
        // fontique derives the family name from the font's own `name` table;
        // custom name overrides are not yet supported here.
        self.font_cx.collection.register_fonts(data.into(), None);
        Ok(())
    }

    /// Resolve a CSS font query to a concrete font + its bytes via fontique.
    fn resolve(&mut self, query: &FontQuery<'_>) -> Result<ResolvedFont, FontError> {
        let families: Vec<QueryFamily> = query.families.iter().map(|&name| css_family_to_query(name)).collect();

        let attrs = Attributes::new(
            stretch_to_width(query.stretch),
            style_to_fontique(query.style),
            weight_to_fontique(query.weight),
        );

        let mut col_clone = self.font_cx.collection.clone();
        let mut q = self.font_cx.collection.query(&mut self.source_cache);
        q.set_families(families);
        q.set_attributes(attrs);

        let mut found: Option<ResolvedFont> = None;
        q.matches_with(|cand| {
            // Extract the inner Arc from fontique's Blob<u8> without copying bytes.
            let (data_arc, _) = cand.blob.clone().into_raw_parts();
            let blob = FontBlob::new(data_arc, cand.index);

            let (fam_id, _) = cand.family;
            let family = col_clone
                .family(fam_id)
                .map(|f| f.name().to_string())
                .unwrap_or_else(|| query.families.first().copied().unwrap_or("sans-serif").to_string());

            found = Some(ResolvedFont {
                family,
                style: query.style,
                weight: query.weight,
                stretch: query.stretch,
                blob,
            });

            QueryStatus::Stop
        });

        found.ok_or_else(|| FontError::FontNotFound(query.families.join(", ")))
    }

    /// Shape `text` into positioned glyph runs, resolving `style.family` first so shaping starts
    /// from the same concrete font that [`FontSystem::measure`] used.
    fn shape(&mut self, text: &str, style: &TextStyle) -> ShapedText {
        if text.is_empty() {
            return ShapedText::empty();
        }
        let families = split_css_families(&style.family);
        let query = FontQuery {
            families: &families,
            style: style.style,
            weight: style.weight,
            stretch: style.stretch,
        };
        let Ok(font) = self.resolve(&query) else {
            return ShapedText::empty();
        };
        self.shape_resolved(text, &font, style)
    }

    /// Measure the bounding box of `text` laid out in `style`, in CSS pixels.
    ///
    /// Resolves the family (mapping generics, appending a `sans-serif` fallback) then lays it out
    /// with Parley and reads the line extents.
    fn measure(&mut self, text: &str, style: &TextStyle) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        let families = split_css_families(&style.family);
        let query = FontQuery {
            families: &families,
            style: style.style,
            weight: style.weight,
            stretch: style.stretch,
        };
        let Ok(resolved) = self.resolve(&query) else {
            return (text.chars().count() as f32 * style.size * 0.5, style.size * 1.2);
        };

        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, style.display_scale, false);
        builder.push_default(parley::StyleProperty::FontSize(style.size));
        builder.push_default(parley::StyleProperty::FontFamily(parley::FontFamily::Source(
            resolved.family.as_str().into(),
        )));
        builder.push_default(parley::StyleProperty::FontWeight(ParleyWeight::new(
            style.weight.0 as f32,
        )));
        builder.push_default(parley::StyleProperty::FontStyle(style_to_parley(style.style)));
        if let Some(lh) = style.line_height {
            builder.push_default(parley::StyleProperty::LineHeight(parley::LineHeight::Absolute(lh)));
        }
        if style.letter_spacing != 0.0 {
            builder.push_default(parley::StyleProperty::LetterSpacing(style.letter_spacing));
        }
        builder.push_default(parley::StyleProperty::Brush(()));

        let mut layout = builder.build(text);
        layout.break_all_lines(Some(style.max_width.unwrap_or(f32::INFINITY)));

        let mut width = 0.0f32;
        let mut height = 0.0f32;
        for line in layout.lines() {
            let lm = line.metrics();
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    width = width.max(run.offset() + run.advance());
                }
            }
            height += lm.line_height;
        }
        (width, height)
    }
}

impl ParleyFontSystem {
    /// Shape `text` with an already-resolved font. Layout parameters (size, line height, wrap
    /// width, letter spacing, display scale) come from `style`; the font identity comes from
    /// `font` — which is why measurement and drawing agree when both go through this path.
    fn shape_resolved(&mut self, text: &str, font: &ResolvedFont, style: &TextStyle) -> ShapedText {
        if text.is_empty() {
            return ShapedText::empty();
        }

        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, style.display_scale, false);
        builder.push_default(parley::StyleProperty::FontSize(style.size));
        builder.push_default(parley::StyleProperty::FontFamily(parley::FontFamily::Source(
            font.family.as_str().into(),
        )));
        builder.push_default(parley::StyleProperty::FontWeight(ParleyWeight::new(
            font.weight.0 as f32,
        )));
        builder.push_default(parley::StyleProperty::FontStyle(style_to_parley(font.style)));
        if let Some(lh) = style.line_height {
            builder.push_default(parley::StyleProperty::LineHeight(parley::LineHeight::Absolute(lh)));
        }
        // Applied during measurement too — shaping without it would draw narrower than the
        // layout box that measurement reserved.
        if style.letter_spacing != 0.0 {
            builder.push_default(parley::StyleProperty::LetterSpacing(style.letter_spacing));
        }
        builder.push_default(parley::StyleProperty::Brush(()));

        let mut layout = builder.build(text);
        layout.break_all_lines(Some(style.max_width.unwrap_or(f32::INFINITY)));
        layout.align(to_parley_alignment(style.align), AlignmentOptions::default());

        let mut runs: Vec<ShapedRun> = Vec::new();
        let mut pen_y = 0.0f32;
        let mut total_width = 0.0f32;
        let mut first_ascent = 0.0f32;
        let mut last_line_height = 0.0f32;
        let mut first_line = true;

        for line in layout.lines() {
            let lm = line.metrics();
            if first_line {
                first_ascent = lm.ascent;
                first_line = false;
            }
            last_line_height = lm.line_height;
            let baseline = lm.ascent;

            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    total_width = total_width.max(run.offset() + run.advance());

                    let run_x = run.offset();
                    let mut pen_x = 0.0f32;

                    let glyphs: Vec<ShapedGlyph> = run
                        .glyphs()
                        .map(|g| {
                            let x = run_x + pen_x + g.x;
                            let y = pen_y + baseline + g.y;
                            pen_x += g.advance;
                            ShapedGlyph { id: g.id, x, y }
                        })
                        .collect();

                    if !glyphs.is_empty() {
                        // Use the run's *actual* font: parley may substitute a fallback for
                        // glyphs the requested family lacks (emoji, CJK, …), and the glyph ids
                        // index into that fallback font — so drawing must use it, not the
                        // originally requested `font`.
                        let prun = run.run();
                        let run_font = prun.font();
                        let (data_arc, _) = run_font.data.clone().into_raw_parts();
                        let run_resolved = ResolvedFont {
                            family: font.family.clone(),
                            style: font.style,
                            weight: font.weight,
                            stretch: font.stretch,
                            blob: FontBlob::new(data_arc, run_font.index),
                        };
                        // Parley metrics are y-up font-space (below baseline = negative); our
                        // convention is positive-down, so the offsets flip sign.
                        let pm = prun.metrics();
                        runs.push(ShapedRun {
                            font: run_resolved,
                            font_size: style.size,
                            x: run_x,
                            baseline: pen_y + baseline,
                            width: run.advance(),
                            metrics: RunMetrics {
                                underline_offset: -pm.underline_offset,
                                underline_size: pm.underline_size,
                                strikethrough_offset: -pm.strikethrough_offset,
                                strikethrough_size: pm.strikethrough_size,
                            },
                            glyphs,
                        });
                    }
                }
            }

            pen_y += lm.line_height;
        }

        ShapedText {
            runs,
            width: total_width,
            height: pen_y,
            line_height: last_line_height,
            ascent: first_ascent,
        }
    }
}

/// Split a CSS `font-family` value (e.g. `Verdana, Geneva, sans-serif`) into individual family
/// names, trimming whitespace and matching quotes. A trailing `sans-serif` generic is appended as
/// an ultimate fallback if the list doesn't already end in a generic, so resolution always has a
/// last resort.
///
/// Passing the whole comma-joined string as a single family name (the old behaviour) never matches
/// an installed family like `Verdana`, so resolution silently fell through to the `sans-serif`
/// generic — picking a different (often thinner) font than the page author intended.
pub fn split_css_families(families: &str) -> Vec<&str> {
    let mut out: Vec<&str> = families
        .split(',')
        .map(|f| f.trim().trim_matches(|c| c == '\'' || c == '"').trim())
        .filter(|f| !f.is_empty())
        .collect();
    if !out.iter().any(|f| f.eq_ignore_ascii_case("sans-serif")) {
        out.push("sans-serif");
    }
    out
}

// Conversion helpers

fn css_family_to_query(name: &str) -> QueryFamily<'_> {
    match name.cow_to_lowercase().as_ref() {
        "sans-serif" => GenericFamily::SansSerif.into(),
        "serif" => GenericFamily::Serif.into(),
        "monospace" | "monospaced" => GenericFamily::Monospace.into(),
        "cursive" => GenericFamily::Cursive.into(),
        "fantasy" => GenericFamily::Fantasy.into(),
        "system-ui" => GenericFamily::SystemUi.into(),
        "ui-sans-serif" => GenericFamily::UiSansSerif.into(),
        "ui-serif" => GenericFamily::UiSerif.into(),
        "ui-monospace" => GenericFamily::UiMonospace.into(),
        "ui-rounded" => GenericFamily::UiRounded.into(),
        _ => QueryFamily::Named(name),
    }
}

fn weight_to_fontique(w: FontWeight) -> parley::fontique::FontWeight {
    parley::fontique::FontWeight::new(w.0 as f32)
}

fn stretch_to_width(s: FontStretch) -> FontWidth {
    FontWidth::from_ratio(s.0)
}

fn style_to_fontique(s: FontStyle) -> parley::fontique::FontStyle {
    match s {
        FontStyle::Normal => parley::fontique::FontStyle::Normal,
        FontStyle::Italic => parley::fontique::FontStyle::Italic,
        FontStyle::Oblique => parley::fontique::FontStyle::Oblique(None),
    }
}

fn style_to_parley(s: FontStyle) -> ParleyStyle {
    match s {
        FontStyle::Normal => ParleyStyle::Normal,
        FontStyle::Italic => ParleyStyle::Italic,
        FontStyle::Oblique => ParleyStyle::Oblique(None),
    }
}

fn to_parley_alignment(align: TextAlign) -> Alignment {
    match align {
        TextAlign::Start => Alignment::Start,
        TextAlign::Center => Alignment::Center,
        TextAlign::End => Alignment::End,
        TextAlign::Justify => Alignment::Justify,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_agrees_with_measure_and_applies_letter_spacing() {
        let mut fs = ParleyFontSystem::new();
        let mut style = TextStyle::new("sans-serif", 16.0);

        let shaped = fs.shape("Hello", &style);
        assert!(!shaped.runs.is_empty(), "expected at least one shaped run");
        let (w, h) = fs.measure("Hello", &style);
        assert!(
            (w - shaped.width).abs() < 0.01 && (h - shaped.height).abs() < 0.01,
            "measure ({w} x {h}) must agree with shape ({} x {})",
            shaped.width,
            shaped.height
        );

        style.letter_spacing = 2.0;
        let spaced = fs.shape("Hello", &style);
        assert!(
            spaced.width > shaped.width,
            "letter-spacing must widen shaping: {} -> {}",
            shaped.width,
            spaced.width
        );
    }

    #[test]
    fn letter_spacing_widens_measurement() {
        let mut fs = ParleyFontSystem::new();
        let mut style = TextStyle::new("sans-serif", 16.0);
        let (base_width, _) = fs.measure("Hello", &style);
        assert!(base_width > 0.0, "expected a non-zero base width");

        style.letter_spacing = 2.0;
        let (spaced_width, _) = fs.measure("Hello", &style);
        assert!(
            spaced_width > base_width,
            "letter-spacing should widen the measurement: {base_width} -> {spaced_width}"
        );
    }
}
