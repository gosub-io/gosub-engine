use cairo::Context;
use gosub_render_pipeline::common::geo::Dimension;
use gosub_render_pipeline::common::media::{MediaId, MediaStore};
use gosub_render_pipeline::painter::commands::rectangle::Rectangle;
use gosub_render_pipeline::tiler::Tile;
use resvg::usvg::Transform;

pub(crate) fn do_paint_svg(
    cr: &Context,
    tile: &Tile,
    rect: &Rectangle,
    media_id: MediaId,
    media_store: &MediaStore,
    dpr: i32,
) {
    log::debug!("Painting SVG: {:?}", media_id);
    let media = media_store.get_svg(media_id);

    let target_dim = rect.rect().dimension();

    // Round the placement to integer CSS pixels. The tile context is scaled by `dpr`, so an
    // integer CSS coordinate maps to an integer device pixel — keeping the glyph/icon on the
    // device grid. Without this the surface lands on a fractional device pixel (very visible
    // at dpr ≥ 2) and looks soft and shifted.
    let dest_x = (rect.rect().x - tile.rect.x).round();
    let dest_y = (rect.rect().y - tile.rect.y).round();

    // Rasterize at physical resolution (CSS size × dpr) so the icon is crisp instead of being
    // upscaled from CSS-pixel resolution by the dpr-scaled context. `set_device_scale(dpr)`
    // then maps the physical surface back to its CSS logical size for placement.
    let phys_w = (target_dim.width as u32 * dpr.max(1) as u32).max(1);
    let phys_h = (target_dim.height as u32 * dpr.max(1) as u32).max(1);
    // The cache stores physical pixels, so key it on the physical dimension (which also
    // encodes dpr) — a dpr change re-renders rather than reusing a stale-resolution bitmap.
    let phys_dim = Dimension::new(phys_w as f64, phys_h as f64);

    {
        let cached = media.svg.rendered.read();
        if cached.dimension == phys_dim && !cached.data.is_empty() {
            paint_surface(cr, &cached.data, phys_w, phys_h, dpr, dest_x, dest_y, media_id);
            return;
        }
    }

    let pixmap_size = media.svg.tree.size().to_int_size();
    let intrinsic_w = pixmap_size.width().max(1) as f32;
    let intrinsic_h = pixmap_size.height().max(1) as f32;
    let sx = phys_w as f32 / intrinsic_w;
    let sy = phys_h as f32 / intrinsic_h;

    let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(phys_w, phys_h) else {
        log::warn!("SVG has zero or invalid dimensions, skipping render");
        return;
    };
    resvg::render(&media.svg.tree, Transform::from_scale(sx, sy), &mut pixmap.as_mut());

    let mut new_data = pixmap.data().to_vec();
    for chunk in new_data.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }

    let mut cached = media.svg.rendered.write();
    cached.data = new_data;
    cached.dimension = phys_dim;

    paint_surface(cr, &cached.data, phys_w, phys_h, dpr, dest_x, dest_y, media_id);
}

/// Wrap the physical-pixel ARGB32 buffer in a Cairo surface, scale it back to CSS logical size
/// via `device_scale`, and paint it at the (grid-aligned) destination.
#[allow(clippy::too_many_arguments)]
fn paint_surface(
    cr: &Context,
    data: &[u8],
    phys_w: u32,
    phys_h: u32,
    dpr: i32,
    dest_x: f64,
    dest_y: f64,
    media_id: MediaId,
) {
    let surface = match cairo::ImageSurface::create_for_data(
        data.to_vec(),
        cairo::Format::ARgb32,
        phys_w as i32,
        phys_h as i32,
        phys_w as i32 * 4,
    ) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Failed to create image surface for SVG {:?}: {:?}", media_id, e);
            return;
        }
    };
    // device_scale maps the physical surface (phys = CSS × dpr) back to CSS logical units, so it
    // places 1:1 on the device grid in the dpr-scaled tile context.
    surface.set_device_scale(dpr.max(1) as f64, dpr.max(1) as f64);

    _ = cr.set_source_surface(&surface, dest_x, dest_y);
    cr.source().set_filter(cairo::Filter::Good);
    _ = cr.paint();
}
