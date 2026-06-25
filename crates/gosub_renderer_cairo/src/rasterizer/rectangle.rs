use crate::rasterizer::brush::set_brush;
use cairo::Context;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::border::BorderStyle;
use gosub_render_pipeline::painter::commands::rectangle::Rectangle;
use gosub_render_pipeline::tiler::Tile;

pub(crate) fn do_paint_rectangle(cr: &Context, tile: &Tile, rectangle: &Rectangle, media_store: &MediaStore) {
    _ = cr.save();

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

    setup_rectangle_path(cr, rectangle);

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
        BorderStyle::Groove => {}
        BorderStyle::Ridge => {}
        BorderStyle::Inset => {}
        BorderStyle::Outset => {}
        BorderStyle::Hidden => {}
    }

    _ = cr.restore();
}

fn setup_rectangle_path(cr: &Context, rect: &Rectangle) {
    let (r_tl, r_tr, r_br, r_bl) = rect.radius_x();

    if r_tl == 0.0 && r_tr == 0.0 && r_br == 0.0 && r_bl == 0.0 {
        cr.rectangle(rect.rect().x, rect.rect().y, rect.rect().width, rect.rect().height);
        return;
    }

    cr.move_to(rect.rect().x + r_tl, rect.rect().y);

    cr.line_to(rect.rect().x + rect.rect().width - r_tr, rect.rect().y);
    cr.arc(
        rect.rect().x + rect.rect().width - r_tr,
        rect.rect().y + r_tr,
        r_tr,
        -0.5 * std::f64::consts::PI,
        0.0,
    );

    cr.line_to(
        rect.rect().x + rect.rect().width,
        rect.rect().y + rect.rect().height - r_br,
    );
    cr.arc(
        rect.rect().x + rect.rect().width - r_br,
        rect.rect().y + rect.rect().height - r_br,
        r_br,
        0.0,
        0.5 * std::f64::consts::PI,
    );

    cr.line_to(rect.rect().x + r_bl, rect.rect().y + rect.rect().height);
    cr.arc(
        rect.rect().x + r_bl,
        rect.rect().y + rect.rect().height - r_bl,
        r_bl,
        0.5 * std::f64::consts::PI,
        std::f64::consts::PI,
    );

    cr.line_to(rect.rect().x, rect.rect().y + r_tl);
    cr.arc(
        rect.rect().x + r_tl,
        rect.rect().y + r_tl,
        r_tl,
        std::f64::consts::PI,
        1.5 * std::f64::consts::PI,
    );

    cr.close_path();
}
