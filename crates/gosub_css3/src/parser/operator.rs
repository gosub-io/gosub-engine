use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssError;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    pub fn parse_operator(&mut self) -> CssResult<Node> {
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

        Err(CssError::with_location(
            format!("Expected operator, got {operator:?}").as_str(),
            self.tokenizer.current_location(),
        ))
    }
}
