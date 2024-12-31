use crate::config::{HasDocument, HasLayouter};
use crate::css3::CssSystem;
use crate::document_handle::DocumentHandle;
use crate::layout::Layouter;
use std::collections::HashMap;
use std::fmt::Debug;

pub trait RenderTree<C: HasLayouter>: Send + 'static {
    type NodeId: Copy;

    type Node: RenderTreeNode<C>;

    fn root(&self) -> Self::NodeId;

    fn get_node(&self, id: Self::NodeId) -> Option<&Self::Node>;

    fn get_node_mut(&mut self, id: Self::NodeId) -> Option<&mut Self::Node>;

    fn get_children(&self, id: Self::NodeId) -> Option<Vec<Self::NodeId>>;

    fn get_layout(&self, id: Self::NodeId) -> Option<&<C::Layouter as Layouter>::Layout>;

    fn from_document(handle: DocumentHandle<C>) -> Self
    where
        C: HasDocument;
}

pub trait RenderTreeNode<C: HasLayouter>: Debug {
    fn props(&self) -> &<C::CssSystem as CssSystem>::PropertyMap;

    fn props_mut(&mut self) -> &mut <C::CssSystem as CssSystem>::PropertyMap;

    fn layout(&self) -> &<C::Layouter as Layouter>::Layout;
    fn layout_mut(&mut self) -> &mut <C::Layouter as Layouter>::Layout;

    fn element_attributes(&self) -> Option<&HashMap<String, String>>;
    fn text_data(&self) -> Option<(&str, Option<&<C::Layouter as Layouter>::TextLayout>)>;
    fn name(&self) -> &str;
}
