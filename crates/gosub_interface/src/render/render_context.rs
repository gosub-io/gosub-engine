use crate::render::render_list::RenderList;
use crate::render::viewport::Viewport;

/// Abstraction over the per-tab state that render backends need.
///
/// Implemented by `gosub_engine::BrowsingContext`. Defined here so the render
/// backend trait does not depend on `gosub_engine` or `gosub_render_pipeline`.
pub trait RenderContext {
    fn viewport(&self) -> &Viewport;
    fn render_list(&self) -> &RenderList;

    /// The viewport-level paint scene for GPU backends, type-erased.
    ///
    /// GPU backends (those returning `true` from `RenderBackend::renders_to_gpu_texture`) render
    /// from this instead of the tile-based `render_list`. The concrete type is
    /// `gosub_render_pipeline::painter::PaintScene`, which this interface crate can't name, so it
    /// is returned as `&dyn Any` and the backend downcasts it. Returns `None` for the CPU path.
    fn paint_scene(&self) -> Option<&dyn core::any::Any> {
        None
    }

    /// Current scroll offset in CSS pixels `(x, y)`. GPU backends translate the scene by the
    /// negation of this so scrolling needs no re-layout. Defaults to `(0, 0)`.
    fn scroll_offset(&self) -> (f64, f64) {
        (0.0, 0.0)
    }
}
