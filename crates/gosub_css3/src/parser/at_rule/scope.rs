use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    pub fn parse_at_rule_scope_prelude(&mut self) -> Result<Node, Error> {
        log::trace!("parse_at_rule_scope_prelude");

        let mut root = None;
        let mut limit = None;

        self.consume_whitespace_comments();

        let t = self.consume_any()?;
        if let TokenType::LParen = t.token_type {
            self.consume_whitespace_comments();
            root = Some(self.parse_selector_list()?);
            self.consume_whitespace_comments();

            self.consume(TokenType::RParen)?;
        }

        if let TokenType::Ident(_value) = t.token_type {
            self.consume_whitespace_comments();
            self.consume_ident("to")?;
            self.consume_whitespace_comments();
            self.consume(TokenType::RParen)?;
            self.consume_whitespace_comments();

            limit = Some(self.parse_selector_list()?);
            self.consume_whitespace_comments();
            self.consume(TokenType::RParen)?;
        }

        Ok(Node::new(NodeType::Scope { root, limit }, t.location))
    }
}
