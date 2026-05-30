use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::texture::TextureId;
use gosub_render_pipeline::common::TextureStore;
use gosub_render_pipeline::painter::commands::PaintCommand;
use gosub_render_pipeline::rasterizer::Rasterable;
use gosub_render_pipeline::tiler::Tile;
use gtk4::cairo;

mod brush;
mod rectangle;
mod svg;
mod text;

pub use gosub_render_pipeline::render::backends::cairo::DEVICE_PIXEL_RATIO;

#[derive(Default)]
pub struct CairoRasterizer {}

impl CairoRasterizer {
    pub fn new() -> CairoRasterizer {
        CairoRasterizer {}
    }
}

impl Rasterable for CairoRasterizer {
    fn rasterize(&self, tile: &Tile, texture_store: &mut TextureStore, media_store: &MediaStore) -> Option<TextureId> {
        let dpr = DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed) as i32;

        // Tile surface is created at physical pixel resolution (CSS pixels × DPR).
        let tile_w = tile.rect.width as i32 * dpr;
        let tile_h = tile.rect.height as i32 * dpr;

        let Ok(mut surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, tile_w, tile_h) else {
            log::error!("Failed to create Cairo image surface");
            return None;
        };

        {
            let Ok(cr) = cairo::Context::new(&surface) else {
                log::error!("Failed to create Cairo context");
                return None;
            };
            // Scale the context so all CSS-pixel coordinates map to physical pixels.
            cr.scale(dpr as f64, dpr as f64);

            for element in &tile.elements {
                for command in &element.paint_commands {
                    match command {
                        PaintCommand::Svg(command) => {
                            svg::do_paint_svg(&cr.clone(), tile, &command.rect, command.media_id, media_store);
                        }
                        PaintCommand::Rectangle(command) => {
                            rectangle::do_paint_rectangle(&cr.clone(), tile, command, media_store);
                        }
                        PaintCommand::Text(command) => {
                            #[cfg(feature = "text_pango")]
                            match text::pango::do_paint_text(&cr.clone(), tile, command, media_store, dpr) {
                                Ok(_) => {}
                                Err(e) => {
                                    log::warn!("Failed to paint text: {:?}", e);
                                }
                            }
                            #[cfg(all(not(feature = "text_pango"), feature = "text_parley"))]
                            match text::parley::do_paint_text(&cr.clone(), tile, command, media_store) {
                                Ok(_) => {}
                                Err(e) => {
                                    log::warn!("Failed to paint text: {:?}", e);
                                }
                            }
                            #[cfg(not(any(feature = "text_pango", feature = "text_parley")))]
                            {
                                let _ = command;
                                log::warn!("No text backend enabled; text will not be rendered");
                            }
                        }
                    }
                }
            }

            surface.flush();
        }

        let w = surface.width() as usize;
        let h = surface.height() as usize;

        let Ok(data) = surface.data() else {
            log::error!("Failed to get Cairo surface data");
            return None;
        };

        let texture_id = texture_store.add(w, h, data.to_vec());

        Some(texture_id)
    }
}
