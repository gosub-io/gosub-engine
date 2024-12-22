use gosub_interface::config::HasDocument;

pub trait Visitor<C: HasDocument> {
    fn document_enter(&mut self, node: &C::Node);
    fn document_leave(&mut self, node: &C::Node);

    fn doctype_enter(&mut self, node: &C::Node);
    fn doctype_leave(&mut self, node: &C::Node);

    fn text_enter(&mut self, node: &C::Node);
    fn text_leave(&mut self, node: &C::Node);

    fn comment_enter(&mut self, node: &C::Node);
    fn comment_leave(&mut self, node: &C::Node);

    fn element_enter(&mut self, node: &C::Node);
    fn element_leave(&mut self, node: &C::Node);
}
