use crate::font::skia::get_skia_paragraph;
use gosub_render_pipeline::common::geo::Dimension;
use gosub_render_pipeline::painter::commands::text::Text;
use vello::kurbo::Affine;
use vello::peniko::{Blob, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality};
use vello::Scene;

#[allow(dead_code)]
pub fn do_paint_text(scene: &mut Scene, cmd: &Text, tile_size: Dimension, affine: Affine) -> Result<(), anyhow::Error> {
    let paragraph = get_skia_paragraph(cmd.text.as_str(), &cmd.font_info, cmd.rect.width, None, 1.00);

    let info = skia_safe::ImageInfo::new(
        skia_safe::ISize::new(tile_size.width as i32, tile_size.height as i32),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Premul,
        None,
    );
    let mut surface = skia_safe::surfaces::raster(&info, None, None)
        .ok_or_else(|| anyhow::anyhow!("Failed to create Skia surface for text rendering"))?;
    let canvas = surface.canvas();

    canvas.clip_rect(
        skia_safe::Rect::new(0.0, 0.0, tile_size.width as f32, tile_size.height as f32),
        None,
        None,
    );
    let c = affine.as_coeffs();
    let matrix = skia_safe::Matrix::new_all(
        c[0] as f32,
        c[2] as f32,
        c[4] as f32,
        c[1] as f32,
        c[3] as f32,
        c[5] as f32,
        0.0,
        0.0,
        1.0,
    );
    canvas.clear(skia_safe::Color::TRANSPARENT);
    canvas.concat(&matrix);
    paragraph.paint(canvas, (cmd.rect.x as f32, cmd.rect.y as f32));

    let Some(peek) = canvas.peek_pixels() else {
        return Err(anyhow::anyhow!("Failed to peek pixels from Skia canvas"));
    };
    let Some(bytes) = peek.bytes() else {
        return Err(anyhow::anyhow!("Failed to get bytes from Skia pixel info"));
    };
    let pixels = bytes.to_vec();

    let blob = Blob::from(pixels);
    let img_data = ImageData {
        data: blob,
        format: ImageFormat::Bgra8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width: tile_size.width as u32,
        height: tile_size.height as u32,
    };
    let img_brush = ImageBrush::new(img_data).with_quality(ImageQuality::High);
    scene.draw_image(img_brush.as_ref(), Affine::IDENTITY);

    Ok(())
}
