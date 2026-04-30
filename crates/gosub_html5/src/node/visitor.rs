use crate::node::node_impl::NodeImpl;

pub trait Visitor {
    fn document_enter(&mut self, node: &NodeImpl);
    fn document_leave(&mut self, node: &NodeImpl);

    fn doctype_enter(&mut self, node: &NodeImpl);
    fn doctype_leave(&mut self, node: &NodeImpl);

    fn text_enter(&mut self, node: &NodeImpl);
    fn text_leave(&mut self, node: &NodeImpl);

    fn comment_enter(&mut self, node: &NodeImpl);
    fn comment_leave(&mut self, node: &NodeImpl);

    fn element_enter(&mut self, node: &NodeImpl);
    fn element_leave(&mut self, node: &NodeImpl);
}
