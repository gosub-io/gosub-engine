use crate::html5::node::data::comment::CommentData;
use crate::html5::node::data::doctype::DocTypeData;
use crate::html5::node::data::document::DocumentData;
use crate::html5::node::data::element::ElementData;
use crate::html5::node::data::text::TextData;

pub trait Visitor<Node> {
    fn document_enter(&mut self, node: &Node, data: &DocumentData);
    fn document_leave(&mut self, node: &Node, data: &DocumentData);

    fn doctype_enter(&mut self, node: &Node, data: &DocTypeData);
    fn doctype_leave(&mut self, node: &Node, data: &DocTypeData);

    fn text_enter(&mut self, node: &Node, data: &TextData);
    fn text_leave(&mut self, node: &Node, data: &TextData);

    fn comment_enter(&mut self, node: &Node, data: &CommentData);
    fn comment_leave(&mut self, node: &Node, data: &CommentData);

    fn element_enter(&mut self, node: &Node, data: &ElementData);
    fn element_leave(&mut self, node: &Node, data: &ElementData);
}
