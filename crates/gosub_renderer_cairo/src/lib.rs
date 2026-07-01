pub mod backend;
pub mod compositor;
pub mod font;
pub mod rasterizer;

pub use backend::{CairoBackend, CairoSurface};
pub use compositor::{CairoCompositor, CairoCompositorConfig};
pub use rasterizer::CairoRasterizer;

/// Initialize GTK and Cairo/Pango font resources on the main thread before any
/// background rendering begins. Required when using the Cairo/Pango backend outside
/// a GTK window (e.g. egui, winit, headless). GTK4-window apps may skip this — GTK is
/// already initialized by their `Application`. On headless systems set `GDK_BACKEND=offscreen`.
///
/// # Errors
/// Returns an error if GTK cannot be initialized (e.g. no display available).
#[cfg(feature = "text_pango")]
pub fn init_gtk_resources() -> anyhow::Result<()> {
    gtk4::init()
        .map_err(|e| anyhow::anyhow!("GTK init failed — on headless systems set GDK_BACKEND=offscreen: {e}"))?;
    crate::font::pango::init();
    Ok(())
}
