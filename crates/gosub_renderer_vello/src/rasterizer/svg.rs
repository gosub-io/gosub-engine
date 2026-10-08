use gosub_render_pipeline::common::geo::Dimension;
use gosub_render_pipeline::common::media::{svg_raster_size, MediaId, MediaStore};
use gosub_render_pipeline::painter::commands::rectangle::Rectangle;
use gosub_render_pipeline::render::backend::PixelFormat;
use resvg::usvg::Transform;
use vello::kurbo::{Affine, Vec2};
use vello::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};

pub(crate) fn do_paint_svg(
    scene: &mut vello::Scene,
    media_id: MediaId,
    rect: &Rectangle,
    affine: Affine,
    media_store: &MediaStore,
) {
    log::debug!("Painting SVG: {:?}", media_id);

    // No SVG and no placeholder to stand in for it, so there is nothing to draw.
    let Some(media) = media_store.get_svg(media_id) else {
        return;
    };
    let r = rect.rect();
    let target_dim = r.dimension();
    // `draw_image` places the image's top-left at the transform's origin, so the box position must
    // be folded in or the SVG lands at the viewport origin. (The rectangle painter instead bakes
    // the position into its shape path, which is why it only needs the bare `affine`.)
    let placement = affine * Affine::translate(Vec2::new(r.x, r.y));

    // `draw_image` paints one unit per pixel, so a raster held below the box size by the pixel
    // budget is scaled back up to it. Within the budget the raster is the box and this is 1.
    let (target_w, target_h) = svg_raster_size(target_dim, 1);
    let raster_dim = Dimension::new(f64::from(target_w), f64::from(target_h));
    let box_w = target_dim.width.floor().max(1.0);
    let box_h = target_dim.height.floor().max(1.0);
    let placement = placement * Affine::scale_non_uniform(box_w / raster_dim.width, box_h / raster_dim.height);

    {
        let cached = media.svg.rendered.read();
        if cached.is_usable(raster_dim, PixelFormat::Rgba8) {
            let image = ImageData {
                data: Blob::from(cached.data.clone()),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::AlphaPremultiplied,
                width: cached.dimension.width as u32,
                height: cached.dimension.height as u32,
            };
            scene.draw_image(&image, placement);
            return;
        }
    }

    let intrinsic = media.svg.tree.size().to_int_size();
    let scale_x = target_w as f32 / intrinsic.width().max(1) as f32;
    let scale_y = target_h as f32 / intrinsic.height().max(1) as f32;
    let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(target_w, target_h) else {
        log::error!(
            "Failed to allocate pixmap for SVG {:?} ({}x{})",
            media_id,
            target_w,
            target_h
        );
        return;
    };
    resvg::render(
        &media.svg.tree,
        Transform::from_scale(scale_x, scale_y),
        &mut pixmap.as_mut(),
    );
    let new_data = pixmap.data().to_vec();

    let mut cached = media.svg.rendered.write();
    cached.store(raster_dim, PixelFormat::Rgba8, new_data);

    let image = ImageData {
        data: Blob::from(cached.data.clone()),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width: cached.dimension.width as u32,
        height: cached.dimension.height as u32,
    };
    scene.draw_image(&image, placement);
}
