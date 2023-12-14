use crate::css3::node::{Node, NodeType};
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    pub fn parse_operator(&mut self) -> Result<Node, Error> {
        log::trace!("parse_operator");

        let loc = self.tokenizer.current_location().clone();

        let operator = self.consume_any()?;
        if let TokenType::Delim(c) = operator.token_type {
            if ['/', '*', ',', ':', '+', '-', '='].contains(&c) {
                return Ok(Node::new(NodeType::Operator(c.to_string()), loc));
            }
        }

        Err(Error::new(
            format!("Expected operator, got {:?}", operator),
            self.tokenizer.current_location().clone(),
        ))
    }
}
