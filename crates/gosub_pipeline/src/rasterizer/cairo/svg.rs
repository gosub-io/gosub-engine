use crate::common::get_media_store;
use crate::common::media::MediaId;
use crate::painter::commands::rectangle::Rectangle;
use crate::tiler::Tile;
use gtk4::cairo::Context;
use resvg::usvg::Transform;

pub(crate) fn do_paint_svg(cr: &Context, _tile: &Tile, rect: &Rectangle, media_id: MediaId) {
    let Ok(binding) = get_media_store().read() else {
        log::warn!("Failed to acquire media store lock, skipping SVG paint");
        return;
    };
    let media = binding.get_svg(media_id);

    let target_dim = rect.rect().dimension();

    {
        let cached = media.svg.rendered.read().unwrap();
        if cached.dimension == target_dim && !cached.data.is_empty() {
            // Cache hit — use existing render
            let surface = gtk4::cairo::ImageSurface::create_for_data(
                cached.data.clone(),
                gtk4::cairo::Format::ARgb32,
                cached.dimension.width as i32,
                cached.dimension.height as i32,
                cached.dimension.width as i32 * 4,
            )
            .unwrap();
            _ = cr.set_source_surface(&surface, 0.0, 0.0);
            _ = cr.paint();
            return;
        }
    }

    // Re-render SVG at the required dimension, then update the cache atomically.
    let pixmap_size = media.svg.tree.size().to_int_size();
    let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(pixmap_size.width(), pixmap_size.height()) else {
        log::warn!("SVG has zero or invalid dimensions, skipping render");
        return;
    };
    resvg::render(&media.svg.tree, Transform::default(), &mut pixmap.as_mut());
    let new_data = pixmap.data().to_vec();

    let mut cached = media.svg.rendered.write().unwrap();
    cached.data = new_data;
    cached.dimension = target_dim;

    let surface = gtk4::cairo::ImageSurface::create_for_data(
        cached.data.clone(),
        gtk4::cairo::Format::ARgb32,
        cached.dimension.width as i32,
        cached.dimension.height as i32,
        cached.dimension.width as i32 * 4,
    )
    .unwrap();

    _ = cr.set_source_surface(&surface, 0.0, 0.0);
    _ = cr.paint();
}
