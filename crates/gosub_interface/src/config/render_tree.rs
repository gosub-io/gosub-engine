use crate::config::HasLayouter;
use crate::font::HasFontManager;
use crate::render_tree::RenderTree;

pub trait HasRenderTree: HasLayouter + HasFontManager {
    type RenderTree: RenderTree<Self>;
}
