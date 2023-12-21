use crate::css3::node::{Node, NodeType};
use crate::css3::{Css3, Error};

impl Css3<'_> {
    pub fn parse_at_rule_charset_prelude(&mut self) -> Result<Node, Error> {
        log::trace!("parse_at_rule_charset");

        let loc = self.tokenizer.current_location().clone();

        let value = self.consume_any_string()?;
        let charset = Node::new(NodeType::String { value }, loc.clone());

        // let block = Node::new(NodeType::Block { children: vec![ charset ] }, loc.clone());
        Ok(charset)
    }
}
