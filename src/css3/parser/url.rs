use crate::css3::node::{Node, NodeType};
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    pub fn parse_url(&mut self) -> Result<Node, Error> {
        log::trace!("parse_url");

        let name = self.consume_function()?;
        if name.to_ascii_lowercase() != "url" {
            return Err(Error::new(
                format!("Expected url, got {:?}", name),
                self.tokenizer.current_location.clone(),
            ));
        }

        let t = self.consume_any()?;
        let url = match t.token_type {
            TokenType::QuotedString(url) => url,
            _ => {
                return Err(Error::new(
                    format!("Expected url, got {:?}", t),
                    self.tokenizer.current_location.clone(),
                ))
            }
        };

        self.consume(TokenType::RParen)?;

        Ok(Node::new(NodeType::Url { url }))
    }
}
