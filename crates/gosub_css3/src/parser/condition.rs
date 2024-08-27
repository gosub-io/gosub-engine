use crate::node::{FeatureKind, Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    pub fn parse_condition(&mut self, kind: FeatureKind) -> Result<Node, Error> {
        log::trace!("parse_condition");

        let loc = self.tokenizer.current_location();

        let mut list = Vec::new();

        loop {
            let t = self.consume_any()?;
            match t.token_type {
                TokenType::Comment(_) | TokenType::Whitespace(_) => {
                    // skip
                    continue;
                }
                TokenType::Url(url) => {
                    list.push(Node::new(NodeType::Url { url }, t.location));
                }
                TokenType::Ident(ident) => {
                    list.push(Node::new(NodeType::Ident { value: ident }, t.location));
                }
                TokenType::LParen => {
                    self.tokenizer.reconsume();

                    let term = match kind {
                        FeatureKind::Media => self.parse_media_feature_or_range(kind.clone()),
                        FeatureKind::Container => self.parse_media_feature_or_range(kind.clone()),
                        FeatureKind::Supports => {
                            panic!("not implemented")
                        }
                    };

                    if term.is_err() {
                        self.consume(TokenType::RParen)?;
                        let res = self.parse_condition(kind.clone())?;
                        self.consume(TokenType::LParen)?;
                        return Ok(res);
                    }

                    list.push(term.unwrap());
                }
                TokenType::Function(_) => {
                    let term = self.parse_feature_function(kind.clone())?;
                    list.push(term);
                }
                _ => {
                    self.tokenizer.reconsume();
                    break;
                }
            }
        }

        if list.is_empty() {
            return Err(Error::new("Expected condition".to_string(), loc));
        }

        Ok(Node::new(NodeType::Condition { list }, loc))
    }
}
