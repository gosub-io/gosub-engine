use gtk4::cairo::Context;
use crate::common::get_media_store;
use crate::common::media::MediaId;
use crate::painter::commands::rectangle::Rectangle;
use resvg::usvg::Transform;
use crate::tiler::Tile;

pub(crate) fn do_paint_svg(cr: &Context, _tile: &Tile, rect: &Rectangle, media_id: MediaId) {
    println!("Painting SVG: {:?}", media_id);
    let binding = get_media_store().read().unwrap();
    let media = binding.get_svg(media_id);

    let lock = media.svg.rendered_dimension.read().unwrap();
    let media_dimension = lock.clone();
    drop(lock);

    // Check if we need to re-render the SVG. This happens when we need a different dimension for the same SVG.
    // With "normal" images, we would just scale the image, but since SVG is vector-based, we want to re-render it from
    // the source. It might be better to either render each dimension into a separate media, or store only an X amount of
    // different dimensions. This is a trade-off between memory and CPU usage.
    if  media_dimension != rect.rect().dimension() {
        let pixmap_size = media.svg.tree.size().to_int_size();
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(pixmap_size.width(), pixmap_size.height()).unwrap();
        resvg::render(&media.svg.tree, Transform::default(), &mut pixmap.as_mut());

        let mut var = media.svg.rendered_data.write().unwrap();
        *var = pixmap.data().to_vec();
        let mut var = media.svg.rendered_dimension.write().unwrap();
        *var = rect.rect().dimension();
    }

    // At this point, we have the SVG rendered to raw image data. We can now render that data onto an image.

    let svg_dimension = media.svg.rendered_dimension.read().unwrap();
    let svg_rendered_data = media.svg.rendered_data.read().unwrap();

    let surface = gtk4::cairo::ImageSurface::create_for_data(
        svg_rendered_data.to_vec(),
        gtk4::cairo::Format::ARgb32,
        svg_dimension.width as i32,
        svg_dimension.height as i32,
        svg_dimension.width as i32 * 4,
    ).unwrap();

    _ = cr.set_source_surface(&surface, 0.0, 0.0);
    _ = cr.paint();
}
