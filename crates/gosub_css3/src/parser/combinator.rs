use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssError;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    pub fn parse_combinator(&mut self) -> CssResult<Node> {
        log::trace!("parse_combinator");
        let t = self.consume_any()?;

        let name = match t.token_type {
            TokenType::Whitespace(_) => " ".to_string(),
            TokenType::Delim('+') => t.to_string(),
            TokenType::Delim('>') => t.to_string(),
            TokenType::Delim('~') => t.to_string(),
            TokenType::Delim('/') => {
                let tn1_is_deep = matches!(&self.tokenizer.lookahead(1).token_type, TokenType::Ident(s) if s == "deep");
                if tn1_is_deep && self.tokenizer.lookahead(2).token_type == TokenType::Delim('/') {
                    "/deep/".to_string()
                } else {
                    let message = format!("Unexpected token {:?}", self.tokenizer.lookahead(1));
                    return Err(CssError::with_location(
                        message.as_str(),
                        self.tokenizer.current_location(),
                    ));
                }
            }
            _ => {
                return Err(CssError::with_location(
                    format!("Unexpected token {t:?}").as_str(),
                    self.tokenizer.current_location(),
                ));
            }
        };

        self.consume_whitespace_comments();

        Ok(Node::new(NodeType::Combinator { value: name }, t.location))
    }
}
