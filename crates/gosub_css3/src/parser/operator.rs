use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    pub fn parse_operator(&mut self) -> Result<Node, Error> {
        log::trace!("parse_operator");

        let loc = self.tokenizer.current_location();

        let operator = self.consume_any()?;
        if let TokenType::Delim(c) = operator.token_type {
            match &c {
                '/' | '*' | ',' | ':' | '+' | '-' | '=' => {
                    return Ok(Node::new(NodeType::Operator(c.to_string()), loc));
                }
                _ => {}
            }
        }

        Err(Error::new(
            format!("Expected operator, got {:?}", operator),
            self.tokenizer.current_location(),
        ))
    }
}
