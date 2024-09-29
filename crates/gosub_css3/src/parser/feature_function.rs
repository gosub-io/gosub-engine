use crate::node::{FeatureKind, Node, NodeType};
use crate::Css3;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    pub fn parse_feature_function(&mut self, _kind: FeatureKind) -> CssResult<Node> {
        log::trace!("parse_feature_function");

        Ok(Node::new(NodeType::FeatureFunction, self.tokenizer.current_location()))
    }
}
