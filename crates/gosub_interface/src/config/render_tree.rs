use crate::config::HasLayouter;
use crate::render_tree::RenderTree;

pub trait HasRenderTree: HasLayouter {
    type RenderTree: RenderTree<Self>;
}
