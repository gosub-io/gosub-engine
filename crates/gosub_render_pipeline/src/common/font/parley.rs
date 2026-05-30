use crate::common::font::FontAlignment;
use parking_lot::Mutex;
use parley::Layout;
use std::sync::OnceLock;

static FONT_CTX: OnceLock<Mutex<parley::FontContext>> = OnceLock::new();
static LAYOUT_CTX: OnceLock<Mutex<parley::LayoutContext<[u8; 4]>>> = OnceLock::new();

pub fn get_font_context() -> parking_lot::MutexGuard<'static, parley::FontContext> {
    FONT_CTX.get_or_init(|| Mutex::new(parley::FontContext::new())).lock()
}

fn get_layout_context() -> parking_lot::MutexGuard<'static, parley::LayoutContext<[u8; 4]>> {
    LAYOUT_CTX
        .get_or_init(|| Mutex::new(parley::LayoutContext::new()))
        .lock()
}

pub fn get_parley_layout(
    text: &str,
    font_family: &str,
    font_size: f64,
    line_height: f64,
    font_weight: i32,
    max_width: f64,
    alignment: FontAlignment,
) -> Layout<[u8; 4]> {
    let display_scale = 1.0_f32;
    let max_advance = (max_width * display_scale as f64) as f32;

    let mut font_ctx = get_font_context();
    let mut layout_ctx = get_layout_context();

    let mut builder = layout_ctx.ranged_builder(&mut font_ctx, text, display_scale, false);
    builder.push_default(parley::style::StyleProperty::FontFamily(
        parley::style::FontFamily::Source(font_family.into()),
    ));
    builder.push_default(parley::style::StyleProperty::LineHeight(
        parley::style::LineHeight::Absolute(line_height as f32),
    ));
    builder.push_default(parley::style::StyleProperty::FontSize(font_size as f32));
    builder.push_default(parley::style::StyleProperty::FontWeight(
        parley::style::FontWeight::new(font_weight as f32),
    ));

    let align = match alignment {
        FontAlignment::Start => parley::Alignment::Start,
        FontAlignment::Center => parley::Alignment::Center,
        FontAlignment::End => parley::Alignment::End,
        FontAlignment::Justify => parley::Alignment::Justify,
    };

    let mut layout: Layout<[u8; 4]> = builder.build(text);
    layout.break_all_lines(Some(max_advance));
    layout.align(align, parley::AlignmentOptions::default());

    layout
}
