use crate::render::render_list::RenderList;
use crate::render::viewport::Viewport;

/// Abstraction over the per-tab state that render backends need.
pub trait RenderContext {
    fn viewport(&self) -> &Viewport;
    fn render_list(&self) -> &RenderList;

    /// The viewport-level paint scene for GPU backends, type-erased.
    fn paint_scene(&self) -> Option<&dyn core::any::Any> {
        None
    }

    /// Current scroll offset in CSS pixels `(x, y)`. GPU backends translate the scene by the
    /// negation of this so scrolling needs no re-layout. Defaults to `(0, 0)`.
    fn scroll_offset(&self) -> (f64, f64) {
        (0.0, 0.0)
    }
}
