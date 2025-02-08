mod chrome;
mod css_system;
mod document;
mod layouter;
mod render;
mod render_tree;
mod tree_drawer;

use crate::font::HasFontManager;
pub use chrome::*;
pub use css_system::*;
pub use document::*;
pub use layouter::*;
pub use render::*;
pub use render_tree::*;
pub use tree_drawer::*;

pub trait ModuleConfiguration:
    Sized
    + HasCssSystem
    + HasDocument
    + HasHtmlParser
    + HasFontManager
    + HasLayouter
    + HasRenderTree
    + HasTreeDrawer
    + HasRenderBackend
    + HasChrome
{
}

pub trait HasDrawComponents: HasRenderTree + HasRenderBackend {}

impl<C: HasRenderTree + HasRenderBackend> HasDrawComponents for C {}

pub trait HasWebComponents: HasChrome + 'static {}

impl<C: HasChrome + 'static> HasWebComponents for C {}
