use gosub_render_pipeline::common::media::{MediaId, MediaStore};
use gosub_render_pipeline::painter::commands::rectangle::Rectangle;
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

    let media = media_store.get_svg(media_id);
    let r = rect.rect();
    let target_dim = r.dimension();
    // `draw_image` carries no geometry of its own — it places the image's top-left at the
    // transform's origin. Fold the element's box position into the affine so the SVG lands at its
    // layout box, not the viewport origin (the rectangle painter bakes the position into the shape
    // path instead, so it only needs the bare `affine`).
    let placement = affine * Affine::translate(Vec2::new(r.x, r.y));

    {
        let cached = media.svg.rendered.read();
        if cached.dimension == target_dim && !cached.data.is_empty() {
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
    let target_w = (target_dim.width as u32).max(1);
    let target_h = (target_dim.height as u32).max(1);
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
    cached.data = new_data;
    cached.dimension = target_dim;

    let image = ImageData {
        data: Blob::from(cached.data.clone()),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width: cached.dimension.width as u32,
        height: cached.dimension.height as u32,
    };
    scene.draw_image(&image, placement);
}
