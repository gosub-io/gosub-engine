use crate::css3::node::{FeatureKind, Node, NodeType};
use crate::css3::{Css3, Error};

impl Css3<'_> {
    pub fn parse_feature_function(&mut self, _kind: FeatureKind) -> Result<Node, Error> {
        log::trace!("parse_feature_function");

        Ok(Node::new(
            NodeType::FeatureFunction,
            self.tokenizer.current_location().clone(),
        ))
    }
}
