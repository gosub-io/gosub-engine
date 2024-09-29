use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    pub fn parse_stylesheet_internal(&mut self) -> CssResult<Option<Node>> {
        log::trace!("parse_stylesheet");

        let loc = self.tokenizer.current_location();

        let mut children = Vec::new();

        while !self.tokenizer.eof() {
            let t = self.consume_any()?;

            match t.token_type {
                TokenType::Eof => {}
                TokenType::Whitespace(_) => {}
                TokenType::Comment(comment) => {
                    if comment.chars().nth(2) == Some('!') {
                        children.push(Node::new(NodeType::Comment { value: comment }, t.location));
                    }
                }
                TokenType::Cdo => {
                    children.push(Node::new(NodeType::Cdo, t.location));
                }
                TokenType::Cdc => {
                    children.push(Node::new(NodeType::Cdc, t.location));
                }
                TokenType::AtKeyword(_keyword) => {
                    self.tokenizer.reconsume();

                    let at_rule = self.parse_at_rule(false)?;
                    if let Some(at_rule_node) = at_rule {
                        children.push(at_rule_node);
                    }
                }
                _ => {
                    self.tokenizer.reconsume();

                    let rule = self.parse_rule()?;
                    if let Some(rule_node) = rule {
                        children.push(rule_node);
                    }
                }
            }
        }

        for t in self.tokenizer.get_tokens() {
            log::trace!("{:?}", t);
        }

        if children.is_empty() {
            return Ok(None);
        }

        Ok(Some(Node::new(NodeType::StyleSheet { children }, loc)))
    }
}
