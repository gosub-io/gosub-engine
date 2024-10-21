use crate::traits::css3::CssSystem;

pub trait RenderTree<C: CssSystem>: Send + 'static {
    type NodeId: Copy;

    type Node: RenderTreeNode<C>;

    fn root(&self) -> Self::NodeId;

    fn get_node(&self, id: Self::NodeId) -> Option<&Self::Node>;

    fn get_node_mut(&mut self, id: Self::NodeId) -> Option<&mut Self::Node>;

    fn get_children(&self, id: Self::NodeId) -> Option<Vec<Self::NodeId>>;
}

pub trait RenderTreeNode<C: CssSystem> {
    fn props(&self) -> &C::PropertyMap;

    fn props_mut(&mut self) -> &mut C::PropertyMap;
}
