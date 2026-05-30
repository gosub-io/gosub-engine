use gosub_render_pipeline::common::media::{MediaId, MediaStore};
use gosub_render_pipeline::painter::commands::rectangle::Rectangle;
use gosub_render_pipeline::tiler::Tile;
use gtk4::cairo::Context;
use resvg::usvg::Transform;

pub(crate) fn do_paint_svg(cr: &Context, tile: &Tile, rect: &Rectangle, media_id: MediaId, media_store: &MediaStore) {
    log::debug!("Painting SVG: {:?}", media_id);
    let media = media_store.get_svg(media_id);

    let target_dim = rect.rect().dimension();
    let dest_x = rect.rect().x - tile.rect.x;
    let dest_y = rect.rect().y - tile.rect.y;

    {
        let cached = media.svg.rendered.read();
        if cached.dimension == target_dim && !cached.data.is_empty() {
            let surface = match gtk4::cairo::ImageSurface::create_for_data(
                cached.data.clone(),
                gtk4::cairo::Format::ARgb32,
                cached.dimension.width as i32,
                cached.dimension.height as i32,
                cached.dimension.width as i32 * 4,
            ) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("Failed to create image surface for cached SVG {:?}: {:?}", media_id, e);
                    return;
                }
            };
            _ = cr.set_source_surface(&surface, dest_x, dest_y);
            _ = cr.paint();
            return;
        }
    }

    let target_w = (target_dim.width as u32).max(1);
    let target_h = (target_dim.height as u32).max(1);

    let pixmap_size = media.svg.tree.size().to_int_size();
    let intrinsic_w = pixmap_size.width().max(1) as f32;
    let intrinsic_h = pixmap_size.height().max(1) as f32;
    let sx = target_w as f32 / intrinsic_w;
    let sy = target_h as f32 / intrinsic_h;

    let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(target_w, target_h) else {
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
    cached.dimension = target_dim;

    let surface = match gtk4::cairo::ImageSurface::create_for_data(
        cached.data.clone(),
        gtk4::cairo::Format::ARgb32,
        cached.dimension.width as i32,
        cached.dimension.height as i32,
        cached.dimension.width as i32 * 4,
    ) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Failed to create image surface for SVG {:?}: {:?}", media_id, e);
            return;
        }
    };

    _ = cr.set_source_surface(&surface, dest_x, dest_y);
    _ = cr.paint();
}
