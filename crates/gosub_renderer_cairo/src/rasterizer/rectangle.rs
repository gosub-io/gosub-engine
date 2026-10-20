use crate::rasterizer::brush::set_brush;
use crate::rasterizer::shadow::paint_shadows;
use cairo::{Context, Operator};
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::painter::commands::border::{per_side_strips, BorderStyle};
use gosub_render_pipeline::painter::commands::brush::Brush;
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

pub(crate) fn do_paint_rectangle(cr: &Context, tile: &Tile, rectangle: &Rectangle, media_store: &MediaStore, dpr: f64) {
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

    paint_shadows(cr, tile, rectangle.shadows(), false, dpr);
    if let Some(brush) = rectangle.background() {
        // A tile that does not repeat paints only where it lies; its padded edge must not show.
        let footprint = match brush {
            Brush::Image(_, Some(t)) => t.footprint(rectangle.rect()),
            _ => None,
        };
        if footprint.is_none_or(|f| f.width > 0.0 && f.height > 0.0) {
            _ = cr.save();
            if let Some(f) = footprint {
                cr.rectangle(f.x, f.y, f.width, f.height);
                cr.clip();
            }
            setup_rectangle_path(cr, rectangle);
            set_brush(cr, brush, rectangle.rect(), media_store);
            _ = cr.fill();
            _ = cr.restore();
        }
    }
    paint_shadows(cr, tile, rectangle.shadows(), true, dpr);

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
            let total = rectangle.border().width() as f64;
            if total >= 3.0 {
                // The path above is inset for the FULL width; each strand needs its own
                // inset (half its own width) or the outer band stays unpainted. Thirds
                // split keeps strands + gap within the declared width.
                let strand = (total / 3.0).floor();
                let gap = total - 2.0 * strand;
                cr.new_path();
                cr.set_line_width(strand);
                setup_inset_path(cr, rectangle, strand / 2.0);
                _ = cr.stroke();
                setup_inset_path(cr, rectangle, strand + gap + strand / 2.0);
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
    let brushes = rectangle.border().brushes();
    let strips = per_side_strips(rect, rectangle.border().widths(), &rectangle.border().styles());

    for (i, side) in strips.iter().enumerate() {
        if side.is_empty() {
            continue;
        }
        cr.new_path();
        for strip in side {
            cr.rectangle(strip.x, strip.y, strip.width, strip.height);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_render_pipeline::common::geo::Rect;
    use gosub_render_pipeline::common::media::MediaType;
    use gosub_render_pipeline::layering::layer::LayerId;
    use gosub_render_pipeline::painter::commands::gradient::Tiling;
    use gosub_render_pipeline::tiler::{TileId, TileState};

    /// A 2x2 opaque red RGBA PNG.
    const RED_PNG: [u8; 74] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00,
        0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d, 0x24, 0x00, 0x00, 0x00,
        0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x84, 0x19, 0x60, 0x0c, 0x00,
        0x47, 0xca, 0x07, 0xf9, 0x67, 0x59, 0x6e, 0xb7, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42,
        0x60, 0x82,
    ];

    #[test]
    fn scaled_single_tile_keeps_its_edge_and_paints_nothing_past_it() {
        let store = MediaStore::new();
        let media_id = store.load_media_from_data(MediaType::Image, &RED_PNG).expect("decodes");
        // The 2x2 image scaled to 20x20 (so bilinear sampling) and placed once at the top-left.
        let tiling = Tiling {
            tile_size: (20.0, 20.0),
            position: (0.0, 0.0),
            repeat: (false, false),
        };
        let rect = Rect::new(0.0, 0.0, 40.0, 40.0);
        let rectangle = Rectangle::new(rect).with_background(Brush::image_tiled(media_id, Some(tiling)));
        let tile = Tile {
            id: TileId::new(0),
            layer_id: LayerId::new(0),
            elements: Vec::new(),
            texture_id: None,
            state: TileState::Dirty,
            rect,
            bgcolor: None,
        };

        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 40, 40).expect("surface");
        {
            let cr = Context::new(&surface).expect("context");
            do_paint_rectangle(&cr, &tile, &rectangle, &store, 1.0);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().expect("pixels");
        // ARGB32 is native-endian; on little-endian hosts alpha is the fourth byte.
        let alpha = |x: usize, y: usize| data[y * stride + x * 4 + 3];
        assert_eq!(alpha(19, 10), 255, "the tile's last column is opaque, not faded");
        assert_eq!(alpha(10, 19), 255, "the tile's last row is opaque, not faded");
        assert_eq!(alpha(20, 10), 0, "nothing right of the tile");
        assert_eq!(alpha(10, 20), 0, "nothing below the tile");
    }
}
