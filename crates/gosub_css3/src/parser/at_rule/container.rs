use crate::node::{FeatureKind, Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    pub fn parse_at_rule_container_prelude(&mut self) -> CssResult<Node> {
        log::trace!("parse_at_rule_container_prelude");

        let mut children = Vec::new();

        let t = self.consume_any()?;
        if let TokenType::Ident(value) = &t.token_type {
            // An optional container name may precede the query condition. The condition
            // keywords are not valid names, so anything else is treated as the name.
            if !["none", "and", "not", "or"].contains(&value.as_str()) {
                children.push(Node::new(NodeType::Ident { value: value.clone() }, t.location));
            }
        } else {
            // No container name: put the token back so it is parsed as part of the condition.
            self.tokenizer.reconsume();
        }

        children.push(self.parse_condition(FeatureKind::Container)?);

        Ok(Node::new(NodeType::Container { children }, t.location))
    }
}
