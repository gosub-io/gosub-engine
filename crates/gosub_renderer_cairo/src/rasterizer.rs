use cairo;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::texture::TextureId;
use gosub_render_pipeline::common::TextureStore;
use gosub_render_pipeline::painter::commands::PaintCommand;
use gosub_render_pipeline::rasterizer::Rasterable;
use gosub_render_pipeline::tiler::Tile;
use gosub_interface::font_system::FontSystem;
use parking_lot::Mutex;
use std::sync::Arc;

#[cfg(feature = "text_pango")]
use crate::font::pango::{get as get_font_system, PangoFontSystem};

mod brush;
mod rectangle;
mod svg;
mod text;

use gosub_render_pipeline::render::DEVICE_PIXEL_RATIO;

pub struct CairoRasterizer {
    /// The engine's shared font system, exposed to the layouter so it measures with the
    /// configured instance. Cairo's own text drawing still goes through Pango (`pango` below).
    config_font_system: Option<Arc<Mutex<dyn FontSystem>>>,
    /// Pango font system used for the actual cairo text drawing.
    #[cfg(feature = "text_pango")]
    pango: Arc<PangoFontSystem>,
}

impl Default for CairoRasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl CairoRasterizer {
    /// Create a rasterizer using the process-wide Pango font system singleton, with no shared
    /// engine font system (the layouter falls back to its own instance for measurement).
    ///
    /// The singleton is populated by [`crate::init_gtk_resources`] (or
    /// [`crate::font::pango::init`]) from the GTK main thread.
    pub fn new() -> Self {
        Self {
            config_font_system: None,
            #[cfg(feature = "text_pango")]
            pango: get_font_system(),
        }
    }

    /// Create a rasterizer that shares the engine's font system (used by the layouter for
    /// measurement). Drawing still goes through the process-wide Pango singleton.
    pub fn with_font_system(font_system: Arc<Mutex<dyn FontSystem>>) -> Self {
        Self {
            config_font_system: Some(font_system),
            #[cfg(feature = "text_pango")]
            pango: get_font_system(),
        }
    }
}

impl Rasterable for CairoRasterizer {
    fn font_system(&self) -> Option<Arc<Mutex<dyn FontSystem>>> {
        self.config_font_system.clone()
    }

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
                            svg::do_paint_svg(&cr.clone(), tile, &command.rect, command.media_id, media_store, dpr);
                        }
                        PaintCommand::Rectangle(command) => {
                            rectangle::do_paint_rectangle(&cr.clone(), tile, command, media_store);
                        }
                        PaintCommand::Text(command) => {
                            #[cfg(feature = "text_pango")]
                            match text::pango::do_paint_text(
                                &cr.clone(),
                                tile,
                                command,
                                media_store,
                                dpr,
                                &self.pango,
                            ) {
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

        let texture_id = texture_store.add(
            w,
            h,
            data.to_vec(),
            gosub_render_pipeline::render::backend::PixelFormat::PreMulArgb32,
        );

        Some(texture_id)
    }
}
