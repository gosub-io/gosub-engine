use crate::common::font::skia::get_skia_paragraph;
use crate::common::geo::Dimension;
use crate::painter::commands::text::Text;
use skia_safe::Vector;
use vello::kurbo::Affine;
use vello::peniko::Blob;
use vello::Scene;

pub fn do_paint_text(scene: &mut Scene, cmd: &Text, tile_size: Dimension, affine: Affine) -> Result<(), anyhow::Error> {
    let paragraph = get_skia_paragraph(cmd.text.as_str(), &cmd.font_info, cmd.rect.width, None, 1.00);

    let mut surface =
        skia_safe::surfaces::raster_n32_premul((tile_size.width as i32, tile_size.height as i32))
            .ok_or_else(|| anyhow::anyhow!("Failed to create Skia surface for text rendering"))?;
    let mut canvas = surface.canvas();

    canvas.clip_rect(
        skia_safe::Rect::new(0.0, 0.0, tile_size.width as f32, tile_size.height as f32),
        None,
        None,
    );
    let transform = affine.translation();
    canvas.translate(Vector::new(transform.x as f32, transform.y as f32));

    canvas.clear(skia_safe::Color::TRANSPARENT);
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
