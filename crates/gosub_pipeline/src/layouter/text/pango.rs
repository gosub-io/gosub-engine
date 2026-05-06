use crate::common::font::pango::{find_available_font, to_pango_weight};
use crate::common::font::{FontAlignment, FontInfo};
use crate::common::geo::Dimension;
use gtk4::cairo::{Context, Format, ImageSurface};
use gtk4::pango::{FontDescription, SCALE};
use pangocairo::functions::{context_set_resolution, create_layout};
use pangocairo::pango::WrapMode;

pub fn get_text_layout(
    text: &str,
    font_info: &FontInfo,
    max_width: f64,
    _dpi_scale_factor: f32,
) -> Result<Dimension, anyhow::Error> {
    let surface = ImageSurface::create(Format::ARgb32, 1, 1)?;
    let cr = Context::new(&surface)?;
    let layout = create_layout(&cr);

    // DPI must be 72 or text renders too large
    context_set_resolution(&layout.context(), 72.0);

    let selected_family = find_available_font(font_info.family.as_str(), &layout.context());
    let mut font_desc = FontDescription::new();
    font_desc.set_family(&selected_family);
    font_desc.set_size((font_info.size * SCALE as f64) as i32);
    font_desc.set_weight(to_pango_weight(font_info.weight as usize));
    layout.set_font_description(Some(&font_desc));

    layout.set_text(text);
    layout.set_width((max_width * SCALE as f64) as i32);
    layout.set_wrap(WrapMode::Word);
    layout.set_spacing(0);
    layout.set_line_spacing(0.0);

    match font_info.alignment {
        FontAlignment::Start => layout.set_alignment(gtk4::pango::Alignment::Left),
        FontAlignment::Center => layout.set_alignment(gtk4::pango::Alignment::Center),
        FontAlignment::End => layout.set_alignment(gtk4::pango::Alignment::Right),
        FontAlignment::Justify => layout.set_alignment(gtk4::pango::Alignment::Left),
    }

    Ok(Dimension {
        width: layout.extents().1.width() as f64 / SCALE as f64,
        height: layout.extents().1.height() as f64 / SCALE as f64,
    })
}
