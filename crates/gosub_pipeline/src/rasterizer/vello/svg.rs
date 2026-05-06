use crate::common::get_media_store;
use crate::common::media::MediaId;
use crate::painter::commands::rectangle::Rectangle;
use resvg::usvg::Transform;
use vello::kurbo::Affine;
use vello::peniko::{Blob, ImageFormat};

pub(crate) fn do_paint_svg(scene: &mut vello::Scene, media_id: MediaId, rect: &Rectangle, affine: Affine) {
    let binding = get_media_store().read().unwrap();
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

    // Re-render and update cache atomically.
    let pixmap_size = media.svg.tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixmap_size.width(), pixmap_size.height()).unwrap();
    resvg::render(&media.svg.tree, Transform::default(), &mut pixmap.as_mut());
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
