use cairo;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::texture::TextureId;
use gosub_render_pipeline::common::TextureStore;
use gosub_render_pipeline::painter::commands::PaintCommand;
use gosub_render_pipeline::rasterizer::Rasterable;
use gosub_render_pipeline::tiler::Tile;
#[cfg(feature = "text_pango")]
use std::sync::Arc;

#[cfg(feature = "text_pango")]
use crate::font::pango::{get as get_font_system, PangoFontSystem};

mod brush;
mod rectangle;
mod svg;
mod text;

pub use gosub_render_pipeline::render::backends::cairo::DEVICE_PIXEL_RATIO;

pub struct CairoRasterizer {
    #[cfg(feature = "text_pango")]
    font_system: Arc<PangoFontSystem>,
}

impl Default for CairoRasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl CairoRasterizer {
    /// Create a rasterizer using the process-wide font system singleton.
    ///
    /// The singleton is populated by [`gosub_engine::init_gtk_resources`] (or
    /// `gosub_renderer_cairo::font::pango::init()`) from the GTK main thread.
    /// If it has not been initialised yet the singleton is created without
    /// system-ui font resolution; `"sans"` is used as the fallback in that case.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "text_pango")]
            font_system: get_font_system(),
        }
    }

    /// Create a rasterizer with an explicitly provided font system.
    ///
    /// Use this when you want to share a pre-initialised `PangoFontSystem`
    /// between multiple rasterizer instances, or in tests where you control
    /// font resolution yourself.
    #[cfg(feature = "text_pango")]
    pub fn with_font_system(font_system: Arc<PangoFontSystem>) -> Self {
        Self { font_system }
    }

    /// Expose the font system so callers can share it with other components.
    #[cfg(feature = "text_pango")]
    pub fn font_system(&self) -> Arc<PangoFontSystem> {
        Arc::clone(&self.font_system)
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
                            match text::pango::do_paint_text(
                                &cr.clone(),
                                tile,
                                command,
                                media_store,
                                dpr,
                                &self.font_system,
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

        let texture_id = texture_store.add(w, h, data.to_vec());

        Some(texture_id)
    }
}
