use crate::font::pango::{to_pango_weight, PangoFontSystem};
use crate::rasterizer::brush::set_brush;
use cairo::{Antialias, Context, Error, Filter, FontOptions, Format, HintMetrics, HintStyle, ImageSurface};
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::text::Text;
use gosub_render_pipeline::tiler::Tile;
use gtk4::pango::SCALE;
use pangocairo::functions::{context_set_resolution, create_layout};
use pangocairo::pango::{AttrInt, AttrList, FontDescription, Underline, WrapMode};

pub(crate) fn do_paint_text(
    cr: &Context,
    tile: &Tile,
    cmd: &Text,
    media_store: &MediaStore,
    dpr: i32,
    font_system: &PangoFontSystem,
) -> Result<(), Error> {
    let surface = create_text_layout(cmd, media_store, dpr, font_system)?;

    cr.save()?;

    // Translate so the tile origin maps to the surface origin.
    // No explicit clip — the surface boundary clips hard at pixel edges.
    cr.translate(-tile.rect.x, -tile.rect.y);

    // Round to integer CSS-pixel boundaries before placing; device_scale handles DPR mapping.
    let x = cmd.rect.x.round();
    let y = cmd.rect.y.round();
    cr.set_source_surface(&surface, x, y)?;
    cr.source().set_filter(Filter::Fast);
    cr.paint()?;
    cr.restore()?;

    Ok(())
}

fn create_text_layout(
    cmd: &Text,
    media_store: &MediaStore,
    dpr: i32,
    font_system: &PangoFontSystem,
) -> Result<ImageSurface, Error> {
    // Physical pixel width = CSS width × DPR.
    let taffy_width = (cmd.rect.width.ceil() as i32 * dpr).max(1);

    // Measure pango's natural (unconstrained) width. If pango's single-line width is only
    // slightly wider than taffy's allocation (metric discrepancy ≤ 20px per DPR), expand to
    // pango's width to avoid a spurious extra wrap. Otherwise, use taffy's width so that long
    // text that taffy already wrapped is not forced back onto one line.
    let probe_surface = ImageSurface::create(Format::ARgb32, 1, 1)?;
    let probe_cr = Context::new(&probe_surface)?;
    let natural_layout = build_pango_layout_unconstrained(&probe_cr, cmd, dpr, font_system);
    let (_, natural_rect) = natural_layout.pixel_extents();
    let pango_natural_width = natural_rect.width().max(1);
    let metric_slack = 20 * dpr;
    let width = if pango_natural_width <= taffy_width + metric_slack {
        pango_natural_width.max(taffy_width)
    } else {
        taffy_width
    };

    // Measure actual pango height at the resolved width.
    let measure_surface = ImageSurface::create(Format::ARgb32, width, 1)?;
    let measure_cr = Context::new(&measure_surface)?;
    let measure_layout = build_pango_layout(&measure_cr, cmd, width, dpr, font_system);
    let (_, logical_rect) = measure_layout.pixel_extents();
    let pango_height = logical_rect.height().max(1);

    let height = pango_height.max((cmd.rect.height.ceil() as i32 * dpr).max(1));

    let surface = ImageSurface::create(Format::ARgb32, width, height)?;

    let cr = Context::new(&surface)?;
    let layout = build_pango_layout(&cr, cmd, width, dpr, font_system);

    // Pango's natural text height (pango_height) is based on font metrics and may be
    // smaller than the CSS line-height that the layout engine allocated (height).
    // Centering the text vertically within the allocated space avoids the text
    // appearing pinned to the top with blank space at the bottom.
    let y_offset = ((height - pango_height) / 2).max(0) as f64;

    set_brush(&cr, &cmd.brush, cmd.rect, media_store);
    cr.move_to(0.0, y_offset);
    pangocairo::functions::show_layout(&cr, &layout);

    // Set device_scale AFTER rendering — only affects source sampling in the tile context,
    // not the physical-pixel drawing above.
    surface.set_device_scale(dpr as f64, dpr as f64);

    Ok(surface)
}

fn build_pango_layout_unconstrained(
    cr: &Context,
    cmd: &Text,
    dpr: i32,
    font_system: &PangoFontSystem,
) -> pangocairo::pango::Layout {
    let layout = build_pango_layout_inner(cr, cmd, dpr, font_system);
    layout.set_width(-1);
    layout
}

fn build_pango_layout(
    cr: &Context,
    cmd: &Text,
    width: i32,
    dpr: i32,
    font_system: &PangoFontSystem,
) -> pangocairo::pango::Layout {
    let layout = build_pango_layout_inner(cr, cmd, dpr, font_system);
    layout.set_width(width * SCALE);
    layout
}

fn build_pango_layout_inner(
    cr: &Context,
    cmd: &Text,
    dpr: i32,
    font_system: &PangoFontSystem,
) -> pangocairo::pango::Layout {
    if let Ok(mut font_opts) = FontOptions::new() {
        font_opts.set_antialias(Antialias::Gray);
        font_opts.set_hint_style(HintStyle::Full);
        font_opts.set_hint_metrics(HintMetrics::On);
        cr.set_font_options(&font_opts);
    }

    let layout = create_layout(cr);
    // 96 DPI matches browser convention (1 CSS px = 1/96 in). FreeType hinting
    // is calibrated for 96 DPI, which gives sharper strokes than 72 DPI.
    // Font size is passed in CSS pixels; convert to points (÷ 96 × 72) so the
    // rendered physical size is unchanged: 13.33 px → 10 pt × (96/72) = 13.33 px.
    context_set_resolution(&layout.context(), 96.0);

    let selected_family = font_system.find_available_font(cmd.font_info.family.as_str(), &layout.context());
    let mut font_desc = FontDescription::new();
    font_desc.set_family(&selected_family);
    // CSS px → pt: multiply by 72/96 = 0.75, then by SCALE for Pango units.
    font_desc.set_size((cmd.font_info.size * dpr as f64 * SCALE as f64 * (72.0 / 96.0)) as i32);
    font_desc.set_weight(to_pango_weight(cmd.font_info.weight as usize));
    if cmd.font_info.slant != 0 {
        font_desc.set_style(pangocairo::pango::Style::Italic);
    }
    layout.set_font_description(Some(&font_desc));

    layout.set_text(cmd.text.as_str());
    layout.set_wrap(WrapMode::Word);
    layout.set_spacing(0);

    if cmd.font_info.underline || cmd.font_info.line_through {
        let attrs = AttrList::new();
        if cmd.font_info.underline {
            attrs.insert(AttrInt::new_underline(Underline::Single));
        }
        if cmd.font_info.line_through {
            attrs.insert(AttrInt::new_strikethrough(true));
        }
        layout.set_attributes(Some(&attrs));
    }

    layout
}
