use gtk4::cairo::{Context, Error, Format, ImageSurface};
use gtk4::pango::{FontDescription, SCALE, Layout};
use pangocairo::functions::{context_set_resolution, create_layout};
use pangocairo::pango::WrapMode;
use crate::common::font::pango::{find_available_font, to_pango_weight};
use crate::common::geo::Dimension;
use crate::layouter::text::Alignment;

/// Retrieves the pango layout for the given text, font family, font size and maximum width.
/// it will wrap any long lines based on the pixels found in width.
pub fn get_text_layout(text: &str, font_family: &str, font_size: f64, font_weight: usize, line_height: f64, max_width: f64, alignment: Alignment) -> Result<Dimension, Error> {
    println!("get_text_layout: text: {}, font_family: {}, font_size: {}, font_weight: {}, line_height: {}, max_width: {}, alignment: {:?}", text, font_family, font_size, font_weight, line_height, max_width, alignment);
    let surface = ImageSurface::create(Format::ARgb32, 1, 1)?;
    let cr = Context::new(&surface)?;
    let layout = create_layout(&cr);

    // @TODO: I need to set the DPI resolution to 72dpi, otherwise the text will be too large
    context_set_resolution(&layout.context(), 72.0);

    let selected_family = find_available_font(font_family, &layout.context());
    let mut font_desc = FontDescription::new();
    font_desc.set_family(&selected_family);
    font_desc.set_size((font_size * SCALE as f64) as i32);
    font_desc.set_weight(to_pango_weight(font_weight));
    layout.set_font_description(Some(&font_desc));

    layout.set_text(text);
    layout.set_width((max_width * SCALE as f64) as i32);

    // @TODO: This should be configurable
    layout.set_wrap(WrapMode::Word);

    layout.set_spacing(0);
    layout.set_line_spacing(0.0);

    match alignment {
        Alignment::Left => layout.set_alignment(gtk4::pango::Alignment::Left),
        Alignment::Center => layout.set_alignment(gtk4::pango::Alignment::Center),
        Alignment::Right => layout.set_alignment(gtk4::pango::Alignment::Right),
    }

    Ok(Dimension {
        width: layout.extents().1.width() as f64 / SCALE as f64,
        height: layout.extents().1.height() as f64 / SCALE as f64,
    })
}