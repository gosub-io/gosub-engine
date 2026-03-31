use std::fmt::Error;
use skia_safe::Vector;
use vello::kurbo::Affine;
use vello::peniko::Blob;
use vello::Scene;
use crate::painter::commands::text::Text;
use crate::common::font::skia::get_skia_paragraph;
use crate::common::geo::Dimension;

pub fn do_paint_text(scene: &mut Scene, cmd: &Text, tile_size: Dimension, affine: Affine) -> Result<(), Error> {
    let paragraph = get_skia_paragraph(cmd.text.as_str(), &cmd.font_info, cmd.rect.width, None, 1.00);

    // Create a (skia) surface to render onto
    // @TODO: THIS IS CPU, NOT GPU!
    let mut surface = skia_safe::surfaces::raster_n32_premul((tile_size.width as i32, tile_size.height as i32)).unwrap();
    let mut canvas = surface.canvas();

    // Clip and translate the canvas so only our tile is painted and 0.0 coordinate is the top left of the tile
    canvas.clip_rect(skia_safe::Rect::new(0.0,0.0, tile_size.width as f32, tile_size.height as f32), None, None);
    let transform = affine.translation();
    canvas.translate(Vector::new(transform.x as f32, transform.y as f32));

    canvas.clear(skia_safe::Color::TRANSPARENT);
    // paragraph.paint(&mut canvas, (-(transform.x - cmd.rect.x) as f32, -(transform.y - cmd.rect.y) as f32));
    paragraph.paint(&mut canvas, (cmd.rect.x as f32, cmd.rect.y as f32));

    // let img = surface.image_snapshot();
    // let data = img.encode_to_data(skia_safe::EncodedImageFormat::PNG).unwrap();
    // let b = data.as_bytes();
    // std::fs::write(format!("text-{}.png", tile.id), b).unwrap();

    // Now, we need to copy the skia surface into a vello scene. It would be very nice if we could find a way
    // to do this without copying the pixels. If we can find the texture-id of the skia canvas/surface, then we
    // might be able to use that directly in the vello scene. For now, we use copy stuff (probably).
    let peek = canvas.peek_pixels().unwrap();
    let pixels = peek.bytes().unwrap().to_vec();
    let blob = Blob::from(pixels);
    let mut img = vello::peniko::Image::new(blob, vello::peniko::ImageFormat::Rgba8, tile_size.width as u32, tile_size.height as u32);
    img.quality = vello::peniko::ImageQuality::High;
    scene.draw_image(&img, Affine::IDENTITY);

    Ok(())
}