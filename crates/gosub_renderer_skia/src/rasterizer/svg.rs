use gosub_render_pipeline::common::geo::Dimension;
use gosub_render_pipeline::common::media::{MediaId, MediaStore};
use gosub_render_pipeline::painter::commands::rectangle::Rectangle;
use gosub_render_pipeline::tiler::Tile;
use resvg::usvg::Transform;
use skia_safe::{images, AlphaType, Canvas, ColorType, Data, ImageInfo, Paint, Rect as SkRect, SamplingOptions};

/// Rasterize an SVG at physical resolution (CSS size × dpr) and blit it onto the tile canvas.
///
/// The tile canvas is already scaled by `dpr` and translated into page space (see
/// `SkiaRasterizer::rasterize`), so we place the image at its page-coordinate rect in CSS units
/// and let that scale map it 1:1 onto device pixels — crisp on HiDPI instead of upscaled from
/// CSS resolution. The rendered physical pixels are cached on the `Svg` (keyed by physical
/// dimension) and shared with the Cairo backend, which uses the same premultiplied-BGRA byte order.
pub fn do_paint_svg(canvas: &Canvas, _tile: &Tile, media_id: MediaId, rect: &Rectangle, media_store: &MediaStore, dpr: i32) {
    let media = media_store.get_svg(media_id);
    let target_dim = rect.rect().dimension();
    let dpr = dpr.max(1) as u32;

    let phys_w = (target_dim.width as u32 * dpr).max(1);
    let phys_h = (target_dim.height as u32 * dpr).max(1);
    // Key the cache on physical dimension (which encodes dpr) so a dpr change re-renders instead
    // of reusing a stale-resolution bitmap.
    let phys_dim = Dimension::new(phys_w as f64, phys_h as f64);

    // Fast path: reuse cached physical pixels when they match the requested dimension.
    {
        let cached = media.svg.rendered.read();
        if cached.dimension == phys_dim && !cached.data.is_empty() {
            blit(canvas, &cached.data, phys_w, phys_h, rect);
            return;
        }
    }

    // Render the SVG tree into a physical-resolution pixmap, then convert tiny_skia's premultiplied
    // RGBA to the premultiplied BGRA byte order Skia (and the shared cache) expect.
    let size = media.svg.tree.size().to_int_size();
    let sx = phys_w as f32 / size.width().max(1) as f32;
    let sy = phys_h as f32 / size.height().max(1) as f32;
    let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(phys_w, phys_h) else {
        log::warn!("SVG {media_id:?} has zero or invalid dimensions, skipping render");
        return;
    };
    resvg::render(&media.svg.tree, Transform::from_scale(sx, sy), &mut pixmap.as_mut());

    let mut data = pixmap.data().to_vec();
    for chunk in data.chunks_exact_mut(4) {
        chunk.swap(0, 2); // RGBA -> BGRA
    }

    let mut cached = media.svg.rendered.write();
    cached.data = data;
    cached.dimension = phys_dim;
    blit(canvas, &cached.data, phys_w, phys_h, rect);
}

/// Wrap a premultiplied-BGRA physical-pixel buffer in a Skia image and draw it into the element's
/// CSS-space rect. The canvas' dpr scale turns that CSS rect back into the physical pixels the
/// buffer holds, so the blit lands 1:1 on device pixels.
fn blit(canvas: &Canvas, data: &[u8], phys_w: u32, phys_h: u32, rect: &Rectangle) {
    let info = ImageInfo::new(
        skia_safe::ISize::new(phys_w as i32, phys_h as i32),
        ColorType::BGRA8888,
        AlphaType::Premul,
        None,
    );
    let row_bytes = phys_w as usize * 4;
    let Some(image) = images::raster_from_data(&info, Data::new_copy(data), row_bytes) else {
        log::warn!("Failed to build Skia image for SVG");
        return;
    };
    let r = rect.rect();
    let dest = SkRect::new(r.x as f32, r.y as f32, (r.x + r.width) as f32, (r.y + r.height) as f32);
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    canvas.draw_image_rect_with_sampling_options(&image, None, dest, SamplingOptions::default(), &paint);
}
