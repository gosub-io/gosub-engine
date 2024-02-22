use crate::node::{FeatureKind, Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    pub fn parse_at_rule_container_prelude(&mut self) -> Result<Node, Error> {
        log::trace!("parse_at_rule_container_prelude");

        let mut children = Vec::new();

        let t = self.consume_any()?;
        if let TokenType::Ident(value) = t.token_type {
            if !["none", "and", "not", "or"].contains(&value.as_str()) {
                children.push(Node::new(NodeType::Ident { value }, t.location.clone()));
            }
        }

        children.push(self.parse_condition(FeatureKind::Container)?);

        Ok(Node::new(
            NodeType::Container { children },
            t.location.clone(),
        ))
    }
}
