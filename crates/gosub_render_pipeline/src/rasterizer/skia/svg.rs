use crate::common::get_media_store;
use crate::common::media::MediaId;
use crate::painter::commands::rectangle::Rectangle;
use crate::tiler::Tile;
use resvg::usvg::Transform;
use skia_safe::{images, AlphaType, ColorType, Data, ISize, ImageInfo};

// At this point we can render an SVG. This is a two-step process: first, we need to render the svg to a pixmap
// for a certain size. Then, the next step is to render that pixmap into an image which is rendered onto the canvas.

// To speed things up, we can render the SVG to a pixmap once and store it in the media store. However, we should
// be able to use the dimension as a caching tag. So if the rect() dimension changes for that SVG, we should re-render
// the SVG to a pixmap and image and store it in the media store.

pub(crate) fn do_paint_svg(
    canvas: &skia_safe::Canvas,
    _tile: &Tile,
    media_id: MediaId,
    rect: &Rectangle,
) {
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
    let img_info = ImageInfo::new(
        ISize::new(svg_dimension.width as i32, svg_dimension.height as i32),
        // ColorType::RGBA8888,
        ColorType::BGRA8888,
        AlphaType::Premul,
        None,
    );

    let svg_rendered_data = media.svg.rendered_data.read().unwrap();
    let data = Data::new_copy(svg_rendered_data.as_slice());

    match images::raster_from_data(&img_info, data, svg_dimension.width as usize * 4) {
        Some(skia_image) => {
            canvas.draw_image(
                &skia_image,
                (rect.rect().x as f32, rect.rect().y as f32),
                None);
        }
        None => {
            println!("Error rendering SVG");
        }
    }
}
