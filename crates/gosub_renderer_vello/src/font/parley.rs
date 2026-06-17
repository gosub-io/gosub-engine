use gosub_render_pipeline::common::font::{FontAlignment, FontInfo};
use parley::{Alignment, AlignmentOptions, FontContext, FontFamily, Layout, LayoutContext, StyleProperty};

/// Build a parley layout for `text` using the shared `font_cx`.
///
/// A fresh `LayoutContext` is created per call because its brush type `[u8; 4]`
/// is render-pipeline-specific and it holds no expensive state — the expensive
/// state (font collection) lives in `font_cx` which is shared.
///
/// `resolved_family` must be the *concrete* family name the font system resolved
/// `font_info.family` to (via `ParleyFontSystem::resolve`). Using it — rather than re-resolving
/// the raw CSS family list here — guarantees the renderer shapes against the exact same font the
/// layouter measured against. Letting Parley re-resolve the list independently can pick a different
/// concrete `sans-serif`, whose wider metrics make multi-word runs re-wrap into a box only tall
/// enough for one line (overlapping the row below).
pub fn get_parley_layout(
    text: &str,
    font_info: &FontInfo,
    resolved_family: &str,
    max_width: f64,
    font_cx: &mut FontContext,
) -> Layout<[u8; 4]> {
    let mut layout_cx: LayoutContext<[u8; 4]> = LayoutContext::new();

    let display_scale = 1.0_f32;
    let max_advance = (max_width * display_scale as f64) as f32;

    let mut builder = layout_cx.ranged_builder(font_cx, text, display_scale, false);

    builder.push_default(StyleProperty::FontFamily(FontFamily::Source(resolved_family.into())));
    builder.push_default(StyleProperty::FontSize(font_info.size as f32));
    builder.push_default(StyleProperty::LineHeight(parley::LineHeight::Absolute(
        font_info.line_height as f32,
    )));
    builder.push_default(StyleProperty::FontWeight(parley::FontWeight::new(
        font_info.weight as f32,
    )));
    if font_info.slant != 0 {
        builder.push_default(StyleProperty::FontStyle(parley::FontStyle::Italic));
    }

    let align = match font_info.alignment {
        FontAlignment::Start => Alignment::Start,
        FontAlignment::Center => Alignment::Center,
        FontAlignment::End => Alignment::End,
        FontAlignment::Justify => Alignment::Justify,
    };

    let mut layout: Layout<[u8; 4]> = builder.build(text);
    layout.break_all_lines(Some(max_advance * 1.01));
    layout.align(align, AlignmentOptions::default());

    layout
}
