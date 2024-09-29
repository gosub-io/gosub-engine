use gosub_shared::traits::css3::CssSystem;

pub trait Visitor<Node: gosub_shared::traits::node::Node<C>, C: CssSystem> {
    fn document_enter(&mut self, node: &Node);
    fn document_leave(&mut self, node: &Node);

    fn doctype_enter(&mut self, node: &Node);
    fn doctype_leave(&mut self, node: &Node);

    fn text_enter(&mut self, node: &Node);
    fn text_leave(&mut self, node: &Node);

    fn comment_enter(&mut self, node: &Node);
    fn comment_leave(&mut self, node: &Node);

    fn element_enter(&mut self, node: &Node);
    fn element_leave(&mut self, node: &Node);
}
