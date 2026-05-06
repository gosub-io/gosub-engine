use crate::common::get_media_store;
use crate::common::media::MediaId;
use crate::painter::commands::rectangle::Rectangle;
use resvg::usvg::Transform;
use vello::kurbo::Affine;
use vello::peniko::{Blob, ImageFormat};

pub(crate) fn do_paint_svg(scene: &mut vello::Scene, media_id: MediaId, rect: &Rectangle, affine: Affine) {
    log::debug!("Painting SVG: {:?}", media_id);
    let Ok(binding) = get_media_store().read() else {
        log::warn!("Failed to acquire media store lock, skipping SVG paint");
        return;
    };
    let media = binding.get_svg(media_id);

    let target_dim = rect.rect().dimension();

    {
        let cached = media.svg.rendered.read().unwrap();
        if cached.dimension == target_dim && !cached.data.is_empty() {
            let data = Blob::from(cached.data.clone());
            let vello_img = vello::peniko::Image::new(
                data,
                ImageFormat::Rgba8,
                cached.dimension.width as u32,
                cached.dimension.height as u32,
            );
            scene.draw_image(&vello_img, affine);
            return;
        }
    }

    // Re-render at the requested display size so cached bytes and dimension stay in sync.
    let intrinsic = media.svg.tree.size().to_int_size();
    let target_w = (target_dim.width as u32).max(1);
    let target_h = (target_dim.height as u32).max(1);
    let scale_x = target_w as f32 / intrinsic.width().max(1) as f32;
    let scale_y = target_h as f32 / intrinsic.height().max(1) as f32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(target_w, target_h).unwrap();
    resvg::render(&media.svg.tree, Transform::from_scale(scale_x, scale_y), &mut pixmap.as_mut());
    let new_data = pixmap.data().to_vec();

    let mut cached = media.svg.rendered.write().unwrap();
    cached.data = new_data;
    cached.dimension = target_dim;

    let data = Blob::from(cached.data.clone());
    let vello_img = vello::peniko::Image::new(
        data,
        ImageFormat::Rgba8,
        cached.dimension.width as u32,
        cached.dimension.height as u32,
    );

    scene.draw_image(&vello_img, affine);
}
