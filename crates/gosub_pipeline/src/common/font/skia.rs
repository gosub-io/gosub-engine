use skia_safe::{FontStyle, Paint};
use skia_safe::textlayout::{Paragraph, ParagraphBuilder, ParagraphStyle, TextStyle};
use crate::common::font::{FontAlignment, FontInfo};

thread_local! {
    static FC: skia_safe::textlayout::FontCollection = {
        let mut fc = skia_safe::textlayout::FontCollection::new();
        fc.set_default_font_manager(skia_safe::FontMgr::new(), None);
        fc
    };
}

pub fn get_skia_paragraph(text: &str, font_info: &FontInfo, max_width: f64, paint: Option<&Paint>, _dpi_scale_factor: f32) -> Paragraph {
    let mut paragraph_style = ParagraphStyle::new();
    paragraph_style.set_text_align(match font_info.alignment {
        FontAlignment::Start => skia_safe::textlayout::TextAlign::Start,
        FontAlignment::Center => skia_safe::textlayout::TextAlign::Center,
        FontAlignment::End => skia_safe::textlayout::TextAlign::End,
        FontAlignment::Justify => skia_safe::textlayout::TextAlign::Justify,
    });
    paragraph_style.set_text_direction(skia_safe::textlayout::TextDirection::LTR);

    let mut paragraph_builder = ParagraphBuilder::new(&paragraph_style, FC.with(|fc| fc.clone()));

    let paint = match paint {
        Some(p) => p.clone(),
        None => Paint::default(),
    };

    let font_size_px = font_info.size;
    let line_height_px = 1.2 * font_size_px;

    let mut ts = TextStyle::new();
    ts.set_foreground_paint(&paint);
    ts.set_font_size(font_size_px as f32);
    ts.set_height(line_height_px as f32);
    ts.set_font_families(&[font_info.family.clone()]);
    ts.set_font_style(FontStyle::new(font_info.weight.into(), font_info.width.into(), to_slant(font_info.slant)));
    paragraph_builder.push_style(&ts);

    paragraph_builder.add_text(text);

    let mut paragraph = paragraph_builder.build();
    paragraph.layout(max_width as f32);

    paragraph
}

fn to_slant(slant: i32) -> skia_safe::font_style::Slant {
    match slant {
        0 => skia_safe::font_style::Slant::Upright,
        1 => skia_safe::font_style::Slant::Italic,
        2 => skia_safe::font_style::Slant::Oblique,
        _ => skia_safe::font_style::Slant::Upright,
    }
}