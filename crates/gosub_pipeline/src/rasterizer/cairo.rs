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

#[cfg(feature = "text_pango")]
use crate::rasterizer::cairo::text::pango::do_paint_text;

pub struct CairoRasterizer {}

impl Default for CairoRasterizer {
    fn default() -> Self {
        Self::new()
    }
}

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
            // Each tile has a number of elements which have paint commands. We need to execute these paint commands in order
            // onto this surface
            let cr = cairo::Context::new(&surface).expect("Failed to create cairo context");

            // Iterate all elements on this tile
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
                            if let Err(e) = do_paint_text(&cr.clone(), tile, command) {
                                log::warn!("Failed to paint text: {:?}", e);
                            }
                            #[cfg(not(feature = "text_pango"))]
                            log::warn!("No text backend enabled for Cairo, skipping text command");
                        }
                    }
                }
            }

            surface.flush();
        }

        let w = surface.width() as usize;
        let h = surface.height() as usize;

        let Ok(data) = surface.data() else {
            panic!("Failed to get surface data");
        };

        let binding = get_texture_store();
        let mut texture_store = binding.write().expect("Failed to get texture store");
        let texture_id = texture_store.add(w, h, data.to_vec());

        Some(texture_id)
    }
}
