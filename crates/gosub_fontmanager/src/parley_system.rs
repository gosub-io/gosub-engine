use cow_utils::CowUtils;
use gosub_interface::font::{FontBlob, FontError, FontStyle};
use gosub_interface::font_system::{
    Confinement, FontQuery, FontStretch, FontSystem, FontWeight, ResolvedFont, RunMetrics, ShapedGlyph, ShapedRun,
    ShapedText, TextAlign, TextStyle,
};
use parley::fontique::{
    Attributes, Blob, Collection, FontInfoOverride, FontWidth, GenericFamily, QueryFamily, QueryStatus, SourceCache,
};
use parley::style::{FontStyle as ParleyStyle, FontWeight as ParleyWeight};
use parley::{Alignment, AlignmentOptions, FontContext, LayoutContext, PositionedLayoutItem};

/// A [`FontSystem`] backed by Parley + Fontique.
///
/// One shared `FontContext`/`LayoutContext` so layout and rendering produce consistent
/// glyph metrics. Wrap in `Arc<Mutex<..>>` and hand the same `Arc` to layouter and backend.
pub struct ParleyFontSystem {
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,
    /// Shares one backing store with `font_cx.source_cache`: Parley prunes that
    /// one every layout (entries idle for 128 layouts are dropped), and a miss
    /// there falls through to the shared store's weak handle rather than to disk
    /// - as long as something still holds the blob strongly (`pinned`).
    source_cache: SourceCache,
    /// Strong handles to every face [`FontSystem::prepare_for_confinement`]
    /// loaded, so pruning can never turn a warmed face back into a file read.
    pinned: Vec<Blob<u8>>,
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

        // Bundled Roboto fallback so rendering works with no system fonts installed.
        font_cx
            .collection
            .register_fonts(gosub_shared::ROBOTO_FONT.to_vec().into(), None);
        fill_missing_monospace(&mut font_cx.collection);

        // Clones of a shared cache share its backing store; see the field docs.
        let source_cache = SourceCache::new_shared();
        font_cx.source_cache = source_cache.clone();

        Self {
            font_cx,
            layout_cx: LayoutContext::new(),
            source_cache,
            pinned: Vec::new(),
        }
    }
}

impl ParleyFontSystem {
    /// Used by `TaffyLayouter` so layout and rendering share one font collection.
    pub fn font_cx_mut(&mut self) -> &mut FontContext {
        &mut self.font_cx
    }
}

impl FontSystem for ParleyFontSystem {
    fn register_font(&mut self, data: Vec<u8>, family_override: Option<&str>) -> Result<(), FontError> {
        // An `@font-face` rule's `font-family` descriptor names the font for the rest of the
        // document, whatever the file's own `name` table says - so it has to override the name
        // fontique would otherwise derive, or `font-family: "MyFace"` never matches the face
        // that was just registered for it.
        let info_override = family_override.map(|family_name| FontInfoOverride {
            family_name: Some(family_name),
            ..Default::default()
        });
        let registered = self.font_cx.collection.register_fonts(data.into(), info_override);
        if registered.is_empty() {
            return Err(FontError::InvalidFont(
                family_override
                    .map(|f| format!("no usable face in font data for '{f}'"))
                    .unwrap_or_else(|| "no usable face in font data".to_string()),
            ));
        }
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

    fn families(&mut self) -> Vec<String> {
        let mut out: Vec<String> = self.font_cx.collection.family_names().map(str::to_string).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Every face is in memory once prepared (see below), so no file access is needed after.
    fn confinement() -> Confinement {
        Confinement::Full
    }

    /// Load every face of every family into the shared source store and pin it.
    fn prepare_for_confinement(&mut self) -> Confinement {
        let names: Vec<String> = self.font_cx.collection.family_names().map(str::to_string).collect();
        for name in names {
            let Some(family) = self.font_cx.collection.family_by_name(&name) else {
                continue;
            };
            for font in family.fonts() {
                if let Some(blob) = font.load(Some(&mut self.source_cache)) {
                    self.pinned.push(blob);
                }
            }
        }
        Confinement::Full
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
    /// Shape `text` with an already-resolved font, so measurement and drawing agree on the
    /// concrete font.
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
        // Must match measurement, or drawing comes out narrower than the reserved layout box.
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
                        // Parley may substitute a fallback font (emoji, CJK); the glyph ids
                        // index into that font, so draw with the run's actual font.
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

/// Split a CSS `font-family` value into trimmed, unquoted family names, appending a
/// `sans-serif` generic as last resort if none is present.
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

/// Fixed-width families to stand in for `monospace` when the platform names none, best
/// first. Android ships the first two.
const MONOSPACE_FALLBACKS: &[&str] = &[
    "Droid Sans Mono",
    "Cutive Mono",
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
];

/// Point `monospace` and `ui-monospace` at an installed fixed-width family when Fontique
/// found none. On Android it looks for a font named `monospace`, which is only the label
/// `fonts.xml` gives Droid Sans Mono, so the generic stayed empty and fell back to the
/// proportional default: `//` in a `<code>` URL ran together into what read as one slash.
/// A platform that names its own (fontconfig, Core Text, DirectWrite) is left alone.
fn fill_missing_monospace(collection: &mut Collection) {
    let Some(id) = MONOSPACE_FALLBACKS.iter().find_map(|name| collection.family_id(name)) else {
        return;
    };
    for generic in [GenericFamily::Monospace, GenericFamily::UiMonospace] {
        if collection.generic_families(generic).next().is_none() {
            collection.set_generic_families(generic, std::iter::once(id));
        }
    }
}

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
    fn an_empty_monospace_generic_gets_an_installed_fixed_width_family() {
        use parley::fontique::CollectionOptions;
        let mut collection = Collection::new(CollectionOptions {
            system_fonts: false,
            ..Default::default()
        });
        // Roboto under a fixed-width family's name: what is checked is which family the
        // generic points at, not the glyphs.
        collection.register_fonts(
            gosub_shared::ROBOTO_FONT.to_vec().into(),
            Some(FontInfoOverride {
                family_name: Some("Droid Sans Mono"),
                ..Default::default()
            }),
        );
        let mono = collection.family_id("Droid Sans Mono").unwrap();
        assert_eq!(collection.generic_families(GenericFamily::Monospace).next(), None);

        fill_missing_monospace(&mut collection);
        assert_eq!(
            collection
                .generic_families(GenericFamily::Monospace)
                .collect::<Vec<_>>(),
            [mono]
        );
        assert_eq!(
            collection
                .generic_families(GenericFamily::UiMonospace)
                .collect::<Vec<_>>(),
            [mono]
        );
    }

    #[test]
    fn a_platform_monospace_generic_is_left_alone() {
        use parley::fontique::CollectionOptions;
        let mut collection = Collection::new(CollectionOptions {
            system_fonts: false,
            ..Default::default()
        });
        let roboto = collection.register_fonts(gosub_shared::ROBOTO_FONT.to_vec().into(), None)[0].0;
        collection.register_fonts(
            gosub_shared::ROBOTO_FONT.to_vec().into(),
            Some(FontInfoOverride {
                family_name: Some("Droid Sans Mono"),
                ..Default::default()
            }),
        );
        collection.set_generic_families(GenericFamily::Monospace, std::iter::once(roboto));

        fill_missing_monospace(&mut collection);
        assert_eq!(
            collection
                .generic_families(GenericFamily::Monospace)
                .collect::<Vec<_>>(),
            [roboto]
        );
    }

    /// `families()` must list every resolvable family: the bundled Roboto (registered in
    /// `new()`) proves registered fonts are included, sortedness proves the ordering contract.
    #[test]
    fn families_lists_registered_fonts_sorted() {
        let mut fs = ParleyFontSystem::new();
        let families = fs.families();
        assert!(families.iter().any(|f| f == "Roboto"), "bundled Roboto must be listed");
        assert!(families.windows(2).all(|w| w[0] < w[1]), "must be sorted and deduped");
    }

    /// An `@font-face` rule's `font-family` descriptor names the font for the rest of the
    /// document, whatever the file's own `name` table says. Registering the bundled Roboto
    /// under a different name must make *that* name resolve - otherwise every
    /// `@font-face { font-family: "MyFace"; src: url(...) }` silently falls back, because
    /// almost no font file calls itself what the CSS calls it.
    #[test]
    fn font_face_family_name_overrides_the_files_own_name() {
        let mut fs = ParleyFontSystem::new();
        fs.register_font(gosub_shared::ROBOTO_FONT.to_vec(), Some("Gosub Parley Alias Test"))
            .expect("registering the bundled font must succeed");

        assert!(
            fs.families().iter().any(|f| f == "Gosub Parley Alias Test"),
            "the @font-face family must be listed under the CSS name"
        );

        let resolved = fs
            .resolve(&FontQuery::new(&["Gosub Parley Alias Test"]))
            .expect("the @font-face family must resolve");
        assert_eq!(
            resolved.family, "Gosub Parley Alias Test",
            "must resolve to the registered face, not a fallback"
        );
        assert!(!resolved.blob.as_u8().is_empty(), "resolved font must carry bytes");
    }

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
