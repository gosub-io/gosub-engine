use gosub_interface::font_system::FontSystem;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::texture::TextureId;
use gosub_render_pipeline::common::TextureStore;
use gosub_render_pipeline::layering::layer::LayerId;
use gosub_render_pipeline::painter::commands::PaintCommand;
use gosub_render_pipeline::rasterizer::Rasterable;
use gosub_render_pipeline::tiler::Tile;
use parking_lot::Mutex;
use skia_safe::{Bitmap, Canvas, Matrix, Paint, Rect, SamplingOptions, TileMode};
use std::sync::Arc;

mod paint;
mod rectangle;
mod svg;
mod text;

pub struct SkiaRasterizer {
    dpi_scale_factor: f32,
    /// The engine's shared font system, exposed to the layouter so it measures with the
    /// configured instance. Skia draws text through `skia_safe`'s own text layout, so this is
    /// not (yet) used for drawing.
    font_system: Option<Arc<Mutex<dyn FontSystem>>>,
}

impl SkiaRasterizer {
    pub fn new(dpi_scale_factor: f32) -> Self {
        Self {
            dpi_scale_factor,
            font_system: None,
        }
    }

    /// Create a rasterizer that shares the engine's font system (used for measurement).
    pub fn with_font_system(dpi_scale_factor: f32, font_system: Arc<Mutex<dyn FontSystem>>) -> Self {
        Self {
            dpi_scale_factor,
            font_system: Some(font_system),
        }
    }
}

impl Rasterable for SkiaRasterizer {
    fn font_system(&self) -> Option<Arc<Mutex<dyn FontSystem>>> {
        self.font_system.clone()
    }

    fn rasterize(&self, tile: &Tile, texture_store: &mut TextureStore, _media_store: &MediaStore) -> Option<TextureId> {
        let width = tile.rect.width as u32;
        let height = tile.rect.height as u32;

        if tile.layer_id != LayerId::new(0) && tile.elements.is_empty() {
            return None;
        }

        let info = skia_safe::ImageInfo::new(
            skia_safe::ISize::new(width as i32, height as i32),
            skia_safe::ColorType::BGRA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let Some(mut surface) = skia_safe::surfaces::raster(&info, None, None) else {
            log::error!("Failed to create Skia surface for tile rasterization");
            return None;
        };

        let canvas = surface.canvas();

        if tile.layer_id == LayerId::new(0) {
            if let Some(bgcolor) = tile.bgcolor {
                canvas.clear(skia_safe::Color4f::new(bgcolor.0, bgcolor.1, bgcolor.2, bgcolor.3));
            } else {
                canvas.clear(skia_safe::Color4f::new(1.0, 1.0, 1.0, 1.0));
            }
        }

        canvas.clip_rect(Rect::new(0.0, 0.0, width as f32, height as f32), None, None);
        canvas.translate((-tile.rect.x as f32, -tile.rect.y as f32));

        for element in &tile.elements {
            for command in &element.paint_commands {
                match command {
                    // The tile path applies layer opacity/anchor at composite, so these scene-only
                    // group markers never appear here — ignore them.
                    PaintCommand::PushLayer { .. } | PaintCommand::PopLayer => {}
                    PaintCommand::Rectangle(command) => {
                        rectangle::do_paint_rectangle(canvas, tile, command);
                    }
                    PaintCommand::Text(command) => {
                        let _ = text::do_paint_text(canvas, tile, command, self.dpi_scale_factor);
                    }
                    PaintCommand::Svg(command) => {
                        svg::do_paint_svg(canvas, tile, command.media_id, &command.rect);
                    }
                }
            }
        }

        let Some(peek) = canvas.peek_pixels() else {
            log::error!("Failed to peek pixels from Skia canvas");
            return None;
        };
        let Some(bytes) = peek.bytes() else {
            log::error!("Failed to get bytes from Skia pixel info");
            return None;
        };
        let pixels = bytes.to_vec();

        let texture_id = texture_store.add(
            width as usize,
            height as usize,
            pixels,
            gosub_render_pipeline::render::backend::PixelFormat::PreMulArgb32,
        );

        Some(texture_id)
    }
}

#[allow(unused)]
const CHECKERED_COLOR_1: skia_safe::Color4f = skia_safe::Color4f::new(1.0, 1.0, 1.0, 1.0);
#[allow(unused)]
const CHECKERED_COLOR_2: skia_safe::Color4f = skia_safe::Color4f::new(1.0, 0.7, 0.7, 1.0);

#[allow(unused)]
fn clear_canvas(canvas: &Canvas, size: (i32, i32)) {
    let tile_size = 8.0;

    let mut bitmap = Bitmap::new();
    bitmap.alloc_n32_pixels((2 * tile_size as i32, 2 * tile_size as i32), true);
    {
        let Some(tmp_canvas) = Canvas::from_bitmap(&bitmap, None) else {
            return;
        };
        tmp_canvas.clear(CHECKERED_COLOR_1);

        let paint = Paint::new(CHECKERED_COLOR_2, None);
        tmp_canvas.draw_rect(Rect::new(tile_size, 0.0, tile_size * 2.0, tile_size), &paint);
        tmp_canvas.draw_rect(Rect::new(0.0, tile_size, tile_size, tile_size * 2.0), &paint);
    }

    let Some(shader) = bitmap.as_image().to_shader(
        (TileMode::Repeat, TileMode::Repeat),
        SamplingOptions::default(),
        Matrix::i(),
    ) else {
        return;
    };

    let mut paint = Paint::default();
    paint.set_shader(shader);
    canvas.draw_rect(Rect::new(0.0, 0.0, size.1 as f32, size.1 as f32), &paint);
}
