use crate::common::font::FontAlignment;
use parley::{Alignment, AlignmentOptions, FontContext, FontFamily, Layout, LayoutContext, LineHeight, StyleProperty};

/// Build a Parley layout for `text` using the shared `font_cx`.
///
/// A local `LayoutContext` is created per call — it is pure scratch space
/// (the expensive state lives in `font_cx`) so allocation cost is negligible.
pub fn get_parley_layout(
    text: &str,
    font_family: &str,
    font_size: f64,
    line_height: f64,
    font_weight: i32,
    max_width: f64,
    alignment: FontAlignment,
    font_cx: &mut FontContext,
) -> Layout<[u8; 4]> {
    let mut layout_cx: LayoutContext<[u8; 4]> = LayoutContext::new();

    let display_scale = 1.0_f32;
    let max_advance = (max_width * display_scale as f64) as f32;

    let mut builder = layout_cx.ranged_builder(font_cx, text, display_scale, false);

    builder.push_default(StyleProperty::FontFamily(FontFamily::Source(font_family.into())));
    builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(line_height as f32)));
    builder.push_default(StyleProperty::FontSize(font_size as f32));
    builder.push_default(StyleProperty::FontWeight(parley::FontWeight::new(font_weight as f32)));

    let align = match alignment {
        FontAlignment::Start => Alignment::Start,
        FontAlignment::Center => Alignment::Center,
        FontAlignment::End => Alignment::End,
        FontAlignment::Justify => Alignment::Justify,
    };

    let mut layout: Layout<[u8; 4]> = builder.build(text);
    layout.break_all_lines(Some(max_advance));
    layout.align(align, AlignmentOptions::default());

    layout
}
