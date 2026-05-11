use crate::common::font::skia::get_skia_paragraph;
use crate::common::geo::Dimension;
use crate::painter::commands::text::Text;
use vello::kurbo::Affine;
use vello::peniko::Blob;
use vello::Scene;

pub fn do_paint_text(scene: &mut Scene, cmd: &Text, tile_size: Dimension, affine: Affine) -> Result<(), anyhow::Error> {
    let paragraph = get_skia_paragraph(cmd.text.as_str(), &cmd.font_info, cmd.rect.width, None, 1.00);

    let mut surface = skia_safe::surfaces::raster_n32_premul((tile_size.width as i32, tile_size.height as i32))
        .ok_or_else(|| anyhow::anyhow!("Failed to create Skia surface for text rendering"))?;
    let mut canvas = surface.canvas();

    canvas.clip_rect(
        skia_safe::Rect::new(0.0, 0.0, tile_size.width as f32, tile_size.height as f32),
        None,
        None,
    );
    // Apply the full affine (scale, rotation, skew, translation) to the canvas.
    // kurbo Affine coeffs: [a, b, c, d, e, f] → x' = ax + cy + e, y' = bx + dy + f
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
    paragraph.paint(&mut canvas, (cmd.rect.x as f32, cmd.rect.y as f32));

    let Some(peek) = canvas.peek_pixels() else {
        return Err(anyhow::anyhow!("Failed to peek pixels from Skia canvas"));
    };
    let Some(bytes) = peek.bytes() else {
        return Err(anyhow::anyhow!("Failed to get bytes from Skia pixel info"));
    };
    let pixels = bytes.to_vec();

    let blob = Blob::from(pixels);
    let mut img = vello::peniko::Image::new(
        blob,
        vello::peniko::ImageFormat::Rgba8,
        tile_size.width as u32,
        tile_size.height as u32,
    );
    img.quality = vello::peniko::ImageQuality::High;
    scene.draw_image(&img, Affine::IDENTITY);

    Ok(())
}
