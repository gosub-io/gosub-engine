use crate::render::render_list::RenderList;
use crate::render::viewport::Viewport;

/// Abstraction over the per-tab state that render backends need.
///
/// Implemented by `gosub_engine::BrowsingContext`. Defined here so the render
/// crate does not depend on `gosub_engine`.
pub trait RenderContext {
    fn viewport(&self) -> &Viewport;
    fn render_list(&self) -> &RenderList;
}
