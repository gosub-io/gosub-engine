use crate::common::font::FontAlignment;
use parking_lot::Mutex;
use parley::{AlignmentOptions, Layout};
use std::sync::OnceLock;

static FONT_CTX: OnceLock<Mutex<parley::FontContext>> = OnceLock::new();
static LAYOUT_CTX: OnceLock<Mutex<parley::LayoutContext>> = OnceLock::new();

pub fn get_font_context() -> parking_lot::MutexGuard<'static, parley::FontContext> {
    FONT_CTX.get_or_init(|| Mutex::new(parley::FontContext::new())).lock()
}

fn get_layout_context() -> parking_lot::MutexGuard<'static, parley::LayoutContext> {
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
    let font_stack = parley::FontStack::from(font_family);

    let display_scale = 1.0;
    let max_advance = (max_width * display_scale) as f32;

    let mut font_ctx = get_font_context();
    let mut layout_ctx = get_layout_context();

    let mut builder = layout_ctx.ranged_builder(&mut font_ctx, text, display_scale as f32);
    builder.push_default(font_stack);
    builder.push_default(parley::StyleProperty::LineHeight(line_height as f32 / font_size as f32));
    builder.push_default(parley::StyleProperty::FontSize(font_size as f32));
    builder.push_default(parley::StyleProperty::FontWeight(parley::FontWeight::new(
        font_weight as f32,
    )));

    let align = match alignment {
        FontAlignment::Start => parley::layout::Alignment::Start,
        FontAlignment::Center => parley::layout::Alignment::Middle,
        FontAlignment::End => parley::layout::Alignment::End,
        FontAlignment::Justify => parley::layout::Alignment::Justified,
    };

    let mut layout: Layout<[u8; 4]> = builder.build(text);
    layout.break_all_lines(Some(max_advance * 1.01));
    layout.align(Some(max_advance), align, AlignmentOptions::default());

    layout
}
