use parley::GenericFamily;
use crate::layouter::text::Alignment;
use std::sync::{Mutex, OnceLock};
use parley::{AlignmentOptions, Layout};

static FONT_CTX: OnceLock<Mutex<parley::FontContext>> = OnceLock::new();
static LAYOUT_CTX: OnceLock<Mutex<parley::LayoutContext>> = OnceLock::new();

pub fn get_font_context() -> std::sync::MutexGuard<'static, parley::FontContext> {
    FONT_CTX
        .get_or_init(|| Mutex::new(parley::FontContext::new()))
        .lock()
        .expect("Failed to lock font context")
}

fn get_layout_context() -> std::sync::MutexGuard<'static, parley::LayoutContext> {
    LAYOUT_CTX
        .get_or_init(|| Mutex::new(parley::LayoutContext::new()))
        .lock()
        .expect("Failed to lock layout context")
}


pub fn get_parley_layout(text: &str, font_family: &str, font_size: f64, line_height: f64, max_width: f64, alignment: Alignment) -> Layout<[u8; 4]> {
    let font_stack = parley::FontStack::from(font_family);

    let display_scale = 1.0;
    let max_advance = (max_width * display_scale) as f32;

    let mut font_ctx = get_font_context();
    let mut layout_ctx = get_layout_context();

    let mut builder = layout_ctx.ranged_builder(&mut font_ctx, text, display_scale as f32);
    builder.push_default(font_stack);
    builder.push_default(parley::StyleProperty::LineHeight(line_height as f32 / font_size as f32));
    builder.push_default(parley::StyleProperty::FontSize(font_size as f32));
    builder.push_default(GenericFamily::SystemUi);

    let align = match alignment {
        Alignment::Start => parley::layout::Alignment::Start,
        Alignment::Middle => parley::layout::Alignment::Middle,
        Alignment::End => parley::layout::Alignment::End,
        Alignment::Justified => parley::layout::Alignment::Justified,
    };

    let mut layout: Layout<[u8; 4]> = builder.build(text);
    layout.break_all_lines(Some(max_advance * 1.01));
    layout.align(Some(max_advance), align, AlignmentOptions::default());

    layout
}