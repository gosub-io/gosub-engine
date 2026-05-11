use crate::common::get_media_store;
use crate::common::media::MediaId;
use crate::painter::commands::rectangle::Rectangle;
use crate::tiler::Tile;
use gtk4::cairo::Context;
use resvg::usvg::Transform;

pub(crate) fn do_paint_svg(cr: &Context, tile: &Tile, rect: &Rectangle, media_id: MediaId) {
    log::debug!("Painting SVG: {:?}", media_id);
    let Ok(binding) = get_media_store().read() else {
        log::warn!("Failed to acquire media store lock, skipping SVG paint");
        return;
    };
    let media = binding.get_svg(media_id);

    let target_dim = rect.rect().dimension();
    let dest_x = rect.rect().x as f64 - tile.rect.x;
    let dest_y = rect.rect().y as f64 - tile.rect.y;

    {
        let Ok(cached) = media.svg.rendered.read() else {
            log::warn!("Failed to acquire SVG rendered lock for {:?}, skipping", media_id);
            return;
        };
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

    // Re-render SVG at the required target dimension with correct scaling.
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

    // tiny_skia produces premultiplied RGBA; Cairo ARgb32 on little-endian expects BGRA.
    let mut new_data = pixmap.data().to_vec();
    for chunk in new_data.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }

    let Ok(mut cached) = media.svg.rendered.write() else {
        log::warn!("Failed to acquire SVG rendered write lock for {:?}, skipping", media_id);
        return;
    };
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
