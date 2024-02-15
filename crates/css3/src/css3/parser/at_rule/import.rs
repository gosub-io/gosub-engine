use crate::css3::node::{Node, NodeType};
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    pub fn parse_at_rule_import_prelude(&mut self) -> Result<Node, Error> {
        log::trace!("parse_at_rule_import");

        let mut children = Vec::new();

        let loc = self.tokenizer.current_location().clone();

        let t = self.consume_any()?;
        match t.token_type {
            TokenType::QuotedString(value) => {
                children.push(Node::new(NodeType::String { value }, loc.clone()));
            }
            TokenType::Url(url) => {
                children.push(Node::new(NodeType::Url { url }, loc.clone()));
            }
            TokenType::Function(name) if name.eq_ignore_ascii_case("url") => {
                children.push(self.parse_function()?);
            }
            _ => {}
        }

        self.consume_whitespace_comments();

        let t = self.consume_any()?;
        match t.token_type {
            TokenType::Ident(value) if value.eq_ignore_ascii_case("layer") => {
                children.push(Node::new(NodeType::Ident { value }, t.location.clone()));
            }
            TokenType::Function(name) if name.eq_ignore_ascii_case("layer") => {
                children.push(self.parse_function()?);
            }
            _ => {}
        }

        self.consume_whitespace_comments();

        let t = self.consume_any()?;
        match t.token_type {
            TokenType::Function(name) if name.eq_ignore_ascii_case("supports") => {
                children.push(self.parse_function()?);
            }
            _ => {}
        }

        self.consume_whitespace_comments();
        let nt = self.tokenizer.lookahead_sc(0);
        match nt.token_type {
            TokenType::Ident(_) => {
                self.tokenizer.reconsume();
                let list = self.parse_media_query_list()?;
                children.push(list);
            }
            TokenType::LParen => {
                self.tokenizer.reconsume();
                let list = self.parse_media_query_list()?;
                children.push(list);
            }
            _ => {}
        }

        Ok(Node::new(NodeType::ImportList { children }, loc.clone()))
    }
}
