use gtk4::cairo::Context;
use crate::painter::commands::border::BorderStyle;
use crate::painter::commands::rectangle::Rectangle;
use crate::rasterizer::cairo::brush::set_brush;
use crate::tiler::Tile;

pub(crate) fn do_paint_rectangle(cr: &Context, tile: &Tile, rectangle: &Rectangle) {
    // Save the context state. This allows us to do clipping and translation without worrying about
    // the state of the context.
    _ = cr.save();

    // Translate the context to the tile's position and clip it.
    cr.translate(-tile.rect.x, -tile.rect.y);
    cr.rectangle(tile.rect.x, tile.rect.y, tile.rect.width, tile.rect.height);
    cr.clip();

    // Create initial rect
    match rectangle.background() {
        Some(brush) => {
            setup_rectangle_path(cr, rectangle);
            set_brush(cr, brush, rectangle.rect());
            _ = cr.fill();
        }
        None => {}
    }

    // Create border
    setup_rectangle_path(cr, rectangle);

    cr.set_line_width(rectangle.border().width() as f64);
    set_brush(cr, &rectangle.border().brush(), rectangle.rect());
    match rectangle.border().style() {
        BorderStyle::None => {
            // No border to draw. Note that the border does not take up any space. This is already
            // calculated in the boxmodel by the layouter.
        }
        BorderStyle::Solid => {
            // Complete solid line
            _ = cr.stroke();
        }
        BorderStyle::Dashed => {
            // 50px dash, 10px gap, 10px dash, 10px gap
            cr.set_dash(&[50.0, 10.0, 10.0, 10.0], 0.0);
            _ = cr.stroke();
        }
        BorderStyle::Dotted => {
            // 10px dash, 10px gap
            cr.set_dash(&[10.0, 10.0], 0.0);
            _ = cr.stroke();
        }
        BorderStyle::Double => {
            if rectangle.border().width() >= 3.0 {
                // The formula  outer border: (N-1) / 2, 1px gap, inner border: (N-1) / 2

                // Outer border
                let width = (rectangle.border().width() / 2.0).floor();
                cr.set_line_width(width as f64);
                _ = cr.stroke();

                let gap_size = 1.0;

                // inner border
                cr.rectangle(
                    rectangle.rect().x + width as f64 + gap_size,
                    rectangle.rect().y + width as f64 + gap_size,
                    rectangle.rect().width - width as f64 - gap_size,
                    rectangle.rect().height - width as f64 - gap_size
                );
                _ = cr.stroke();
            } else {
                // When the width is less than 3.0, we just draw a single line as there is no room for
                // a double border
                _ = cr.stroke();
            }
        }
        BorderStyle::Groove => {}
        BorderStyle::Ridge => {}
        BorderStyle::Inset => {}
        BorderStyle::Outset => {}
        BorderStyle::Hidden => {
            // Don't display anything. But the border still takes up space. This is already
            // calculated in the boxmodel by the layouter.
        }
    }

    // cr.rectangle(rectangle.rect().x, rectangle.rect().y, rectangle.rect().width, rectangle.rect().height);

    // Restore the context state
    _ = cr.restore();
}


/// Creates a cairo rectangle with either sharp or rounded corners. Does not fill or stroke the path.
fn setup_rectangle_path(cr: &Context, rect: &Rectangle) {
    let (r_tl, r_tr, r_br, r_bl) = rect.radius();

    if r_tl == 0.0 && r_tr == 0.0 && r_br == 0.0 && r_bl == 0.0 {
        cr.rectangle(rect.rect().x, rect.rect().y, rect.rect().width, rect.rect().height);
        return;
    }

    cr.move_to(rect.rect().x + r_tl, rect.rect().y);

    cr.line_to(rect.rect().x + rect.rect().width - r_tr, rect.rect().y);
    cr.arc(rect.rect().x + rect.rect().width - r_tr, rect.rect().y + r_tr, r_tr, -0.5 * std::f64::consts::PI, 0.0);

    cr.line_to(rect.rect().x + rect.rect().width, rect.rect().y + rect.rect().height - r_br);
    cr.arc(rect.rect().x + rect.rect().width - r_br, rect.rect().y + rect.rect().height - r_br, r_br, 0.0, 0.5 * std::f64::consts::PI);

    cr.line_to(rect.rect().x + r_bl, rect.rect().y + rect.rect().height);
    cr.arc(rect.rect().x + r_bl, rect.rect().y + rect.rect().height - r_bl, r_bl, 0.5 * std::f64::consts::PI, std::f64::consts::PI);

    cr.line_to(rect.rect().x, rect.rect().y + r_tl);
    cr.arc(rect.rect().x + r_tl, rect.rect().y + r_tl, r_tl, std::f64::consts::PI, 1.5 * std::f64::consts::PI);

    cr.close_path();
}