use crate::common::get_texture_store;
use crate::common::texture::TextureId;
use crate::painter::commands::PaintCommand;
use crate::rasterizer::Rasterable;
use crate::tiler::Tile;
use gtk4::cairo;

mod brush;
mod rectangle;
mod svg;
mod text;

#[derive(Default)]
pub struct CairoRasterizer {}

impl CairoRasterizer {
    pub fn new() -> CairoRasterizer {
        CairoRasterizer {}
    }
}

impl Rasterable for CairoRasterizer {
    fn rasterize(&self, tile: &Tile) -> Option<TextureId> {
        let mut surface =
            cairo::ImageSurface::create(cairo::Format::ARgb32, tile.rect.width as i32, tile.rect.height as i32)
                .expect("Failed to create image surface");

        {
            let cr = cairo::Context::new(&surface).expect("Failed to create cairo context");

            for element in &tile.elements {
                for command in &element.paint_commands {
                    match command {
                        PaintCommand::Svg(command) => {
                            svg::do_paint_svg(&cr.clone(), tile, &command.rect, command.media_id);
                        }
                        PaintCommand::Rectangle(command) => {
                            rectangle::do_paint_rectangle(&cr.clone(), tile, command);
                        }
                        PaintCommand::Text(command) => {
                            #[cfg(feature = "text_pango")]
                            match text::pango::do_paint_text(&cr.clone(), tile, command) {
                                Ok(_) => {}
                                Err(e) => {
                                    log::warn!("Failed to paint text: {:?}", e);
                                }
                            }
                            #[cfg(all(not(feature = "text_pango"), feature = "text_parley"))]
                            match text::parley::do_paint_text(&cr.clone(), tile, command) {
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

        let binding = get_texture_store();
        let mut texture_store = binding.write().expect("Failed to get texture store");
        let texture_id = texture_store.add(w, h, data.to_vec());

        Some(texture_id)
    }
}
