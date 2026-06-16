use gosub_interface::font::{FontError, FontStyle as CssFontStyle};
use gosub_interface::font_system::{FontSystem, TextStyle as GosubTextStyle};
use gosub_render_pipeline::common::font::{FontAlignment, FontInfo};
use skia_safe::textlayout::{Paragraph, ParagraphBuilder, ParagraphStyle, TextStyle};
use skia_safe::{FontStyle, Paint};
use std::any::Any;

thread_local! {
    static FC: skia_safe::textlayout::FontCollection = {
        let mut fc = skia_safe::textlayout::FontCollection::new();
        fc.set_default_font_manager(skia_safe::FontMgr::new(), None);
        fc
    };
}

/// A [`FontSystem`] backed by Skia's `skia_safe` text layout.
///
/// Opaque / backend-coupled like Pango: it measures (and the Skia rasterizer draws) through the
/// same thread-local [`FontCollection`], so measurement matches what Skia paints. Lives in the
/// Skia backend crate for the same reason Pango lives in the Cairo crate — it's tied to its
/// renderer and isn't a portable glyph emitter.
#[derive(Debug, Default)]
pub struct SkiaFontSystem;

impl FontSystem for SkiaFontSystem {
    fn register_font(&mut self, _data: Vec<u8>, _family_override: Option<&str>) -> Result<(), FontError> {
        // Skia resolves fonts via its FontMgr; injecting @font-face bytes into the thread-local
        // FontCollection at runtime is a follow-up.
        log::warn!("SkiaFontSystem::register_font is unsupported; the font was ignored");
        Ok(())
    }

    fn measure(&mut self, text: &str, style: &GosubTextStyle) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        FC.with(|fc| {
            let paragraph_style = ParagraphStyle::new();
            let mut builder = ParagraphBuilder::new(&paragraph_style, fc.clone());

            let mut ts = TextStyle::new();
            ts.set_font_size(style.size);
            ts.set_font_families(std::slice::from_ref(&style.family));
            ts.set_font_style(FontStyle::new(
                skia_safe::font_style::Weight::from(style.weight.0 as i32),
                skia_safe::font_style::Width::NORMAL,
                match style.style {
                    CssFontStyle::Normal => skia_safe::font_style::Slant::Upright,
                    CssFontStyle::Italic => skia_safe::font_style::Slant::Italic,
                    CssFontStyle::Oblique => skia_safe::font_style::Slant::Oblique,
                },
            ));
            builder.push_style(&ts);
            builder.add_text(text);

            let mut paragraph = builder.build();
            // `None` = no wrap; use a large finite advance (Skia dislikes INFINITY).
            paragraph.layout(style.max_width.unwrap_or(1.0e9));
            (paragraph.longest_line(), paragraph.height())
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[allow(dead_code)]
pub fn get_skia_paragraph(
    text: &str,
    font_info: &FontInfo,
    max_width: f64,
    paint: Option<&Paint>,
    _dpi_scale_factor: f32,
) -> Paragraph {
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
    ts.set_font_families(std::slice::from_ref(&font_info.family));
    ts.set_font_style(FontStyle::new(
        font_info.weight.into(),
        font_info.width.into(),
        to_slant(font_info.slant),
    ));
    paragraph_builder.push_style(&ts);

    paragraph_builder.add_text(text);

    let mut paragraph = paragraph_builder.build();
    paragraph.layout(max_width as f32);

    paragraph
}

#[allow(dead_code)]
fn to_slant(slant: i32) -> skia_safe::font_style::Slant {
    match slant {
        0 => skia_safe::font_style::Slant::Upright,
        1 => skia_safe::font_style::Slant::Italic,
        2 => skia_safe::font_style::Slant::Oblique,
        _ => skia_safe::font_style::Slant::Upright,
    }
}
