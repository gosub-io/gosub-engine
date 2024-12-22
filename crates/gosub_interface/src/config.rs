mod css_system;
mod document;
mod layouter;
mod render;
mod render_tree;
mod tree_drawer;

pub use css_system::*;
pub use document::*;
pub use layouter::*;
pub use render::*;
pub use render_tree::*;
pub use tree_drawer::*;

pub trait ModuleConfiguration:
    Sized + HasCssSystem + HasDocument + HasHtmlParser + HasLayouter + HasRenderTree + HasTreeDrawer + HasRenderBackend
{
}

pub trait HasDrawComponents: HasRenderTree + HasRenderBackend {}

impl<C: HasRenderTree + HasRenderBackend> HasDrawComponents for C {}
