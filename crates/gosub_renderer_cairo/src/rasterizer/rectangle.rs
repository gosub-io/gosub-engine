use crate::rasterizer::brush::set_brush;
use cairo::{Context, Operator};
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::border::BorderStyle;
use gosub_render_pipeline::painter::commands::rectangle::{BlendMode, Rectangle};
use gosub_render_pipeline::tiler::Tile;

/// CSS `mix-blend-mode` → cairo compositing operator. The operator blends against the tile
/// surface content already painted beneath the element.
fn to_cairo_operator(mode: BlendMode) -> Operator {
    match mode {
        BlendMode::Normal => Operator::Over,
        BlendMode::Multiply => Operator::Multiply,
        BlendMode::Screen => Operator::Screen,
        BlendMode::Overlay => Operator::Overlay,
        BlendMode::Darken => Operator::Darken,
        BlendMode::Lighten => Operator::Lighten,
        BlendMode::ColorDodge => Operator::ColorDodge,
        BlendMode::ColorBurn => Operator::ColorBurn,
        BlendMode::HardLight => Operator::HardLight,
        BlendMode::SoftLight => Operator::SoftLight,
        BlendMode::Difference => Operator::Difference,
        BlendMode::Exclusion => Operator::Exclusion,
        BlendMode::Hue => Operator::HslHue,
        BlendMode::Saturation => Operator::HslSaturation,
        BlendMode::Color => Operator::HslColor,
        BlendMode::Luminosity => Operator::HslLuminosity,
    }
}

pub(crate) fn do_paint_rectangle(cr: &Context, tile: &Tile, rectangle: &Rectangle, media_store: &MediaStore) {
    _ = cr.save();

    // Element-level mix-blend-mode; cr.restore() at the end returns the operator to Over.
    if rectangle.blend_mode() != BlendMode::Normal {
        cr.set_operator(to_cairo_operator(rectangle.blend_mode()));
    }

    // Translate so the tile origin maps to the surface origin.
    // No explicit clip: Cairo's image surface boundary clips to exact pixel boundaries
    // without anti-aliasing, preventing the semi-transparent edge pixels that the
    // old cr.clip() produced and caused visible seams at tile borders.
    // cr.clip() also cleared the current path; replace that with an explicit new_path()
    // so setup_rectangle_path always starts from a clean slate.
    cr.translate(-tile.rect.x, -tile.rect.y);
    cr.new_path();

    if let Some(brush) = rectangle.background() {
        setup_rectangle_path(cr, rectangle);
        set_brush(cr, brush, rectangle.rect(), media_store);
        _ = cr.fill();
    }

    // Per-side borders (e.g. `border-bottom: 1px solid …`) cannot be expressed as a single
    // stroked rectangle, so draw each visible side as its own filled edge. The uniform path
    // below keeps handling equal-width/style borders (with dashes, double, radius, etc.).
    if !rectangle.border().is_uniform() {
        paint_per_side_border(cr, rectangle, media_store);
        _ = cr.restore();
        return;
    }

    // Stroke a path inset by half the border width so the whole border lies
    // inside the border box (see setup_inset_path).
    setup_inset_path(cr, rectangle, rectangle.border().width() as f64 / 2.0);

    cr.set_line_width(rectangle.border().width() as f64);
    set_brush(cr, &rectangle.border().brush(), rectangle.rect(), media_store);
    match rectangle.border().style() {
        BorderStyle::None => {}
        BorderStyle::Solid => {
            _ = cr.stroke();
        }
        BorderStyle::Dashed => {
            let w = rectangle.border().width() as f64;
            let dash = (w * 3.0).max(3.0);
            cr.set_dash(&[dash, dash], 0.0);
            _ = cr.stroke();
        }
        BorderStyle::Dotted => {
            let w = rectangle.border().width() as f64;
            cr.set_dash(&[w, w], 0.0);
            _ = cr.stroke();
        }
        BorderStyle::Double => {
            if rectangle.border().width() >= 3.0 {
                let width = (rectangle.border().width() / 2.0).floor();
                cr.set_line_width(width as f64);
                _ = cr.stroke();

                let gap_size = 1.0;

                cr.rectangle(
                    rectangle.rect().x + width as f64 + gap_size,
                    rectangle.rect().y + width as f64 + gap_size,
                    rectangle.rect().width - 2.0 * (width as f64 + gap_size),
                    rectangle.rect().height - 2.0 * (width as f64 + gap_size),
                );
                _ = cr.stroke();
            } else {
                _ = cr.stroke();
            }
        }
        // 3D border styles (groove/ridge/inset/outset) are not yet rendered with their
        // light/dark two-tone effect. Fall back to a solid stroke so the border is at
        // least visible (matches the Skia rasterizer). This is what makes e.g. the
        // 1px-inset default `<hr>` render as a visible line.
        BorderStyle::Groove | BorderStyle::Ridge | BorderStyle::Inset | BorderStyle::Outset => {
            _ = cr.stroke();
        }
        BorderStyle::Hidden => {}
    }

    _ = cr.restore();
}

/// Paints a non-uniform border by filling each visible side as a solid edge rectangle.
/// Side order is `[top, right, bottom, left]`. Dashed/dotted/double styles fall back to a
/// solid fill per side, which is the common case for single-side borders.
fn paint_per_side_border(cr: &Context, rectangle: &Rectangle, media_store: &MediaStore) {
    let rect = rectangle.rect();
    let widths = rectangle.border().widths();
    let styles = rectangle.border().styles();
    let brushes = rectangle.border().brushes();

    // (x, y, w, h) for each side's edge rectangle.
    let edges = [
        (rect.x, rect.y, rect.width, widths[0] as f64), // top
        (
            rect.x + rect.width - widths[1] as f64,
            rect.y,
            widths[1] as f64,
            rect.height,
        ), // right
        (
            rect.x,
            rect.y + rect.height - widths[2] as f64,
            rect.width,
            widths[2] as f64,
        ), // bottom
        (rect.x, rect.y, widths[3] as f64, rect.height), // left
    ];

    for i in 0..4 {
        if widths[i] <= 0.0 || styles[i].is_invisible() {
            continue;
        }
        let (x, y, w, h) = edges[i];
        cr.new_path();
        cr.rectangle(x, y, w, h);
        set_brush(cr, &brushes[i], rect, media_store);
        _ = cr.fill();
    }
}

fn setup_rectangle_path(cr: &Context, rect: &Rectangle) {
    setup_inset_path(cr, rect, 0.0);
}

/// Builds the rectangle path inset by `inset` on every side. Cairo centers a
/// stroke's pen on the path, so a border of width `w` must stroke a path inset
/// by `w / 2` to stay entirely inside the border box (CSS borders never paint
/// outside it - a centered stroke bleeds into the neighbouring element and
/// gets painted over, which visibly halved collapsed table borders).
fn setup_inset_path(cr: &Context, rect: &Rectangle, inset: f64) {
    let (r_tl, r_tr, r_br, r_bl) = rect.radius_x();

    let x = rect.rect().x + inset;
    let y = rect.rect().y + inset;
    let width = (rect.rect().width - 2.0 * inset).max(0.0);
    let height = (rect.rect().height - 2.0 * inset).max(0.0);
    let r_tl = (r_tl - inset).max(0.0);
    let r_tr = (r_tr - inset).max(0.0);
    let r_br = (r_br - inset).max(0.0);
    let r_bl = (r_bl - inset).max(0.0);

    if r_tl == 0.0 && r_tr == 0.0 && r_br == 0.0 && r_bl == 0.0 {
        cr.rectangle(x, y, width, height);
        return;
    }

    cr.move_to(x + r_tl, y);

    cr.line_to(x + width - r_tr, y);
    cr.arc(x + width - r_tr, y + r_tr, r_tr, -0.5 * std::f64::consts::PI, 0.0);

    cr.line_to(x + width, y + height - r_br);
    cr.arc(
        x + width - r_br,
        y + height - r_br,
        r_br,
        0.0,
        0.5 * std::f64::consts::PI,
    );

    cr.line_to(x + r_bl, y + height);
    cr.arc(
        x + r_bl,
        y + height - r_bl,
        r_bl,
        0.5 * std::f64::consts::PI,
        std::f64::consts::PI,
    );

    cr.line_to(x, y + r_tl);
    cr.arc(
        x + r_tl,
        y + r_tl,
        r_tl,
        std::f64::consts::PI,
        1.5 * std::f64::consts::PI,
    );

    cr.close_path();
}
