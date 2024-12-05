use crate::traits::config::HasLayouter;
use crate::traits::render_tree::RenderTree;

pub trait HasRenderTree: HasLayouter {
    type RenderTree: RenderTree<Self>;
}
