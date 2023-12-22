use crate::css3::node::{FeatureKind, Node, NodeType};
use crate::css3::{Css3, Error};
use crate::css3::tokenizer::TokenType;

impl Css3<'_> {

    pub fn parse_supports_condition(&mut self) -> Result<Node, Error> {
        log::trace!("parse_supports_condition");

        let loc = self.tokenizer.current_location().clone();

        self.consume(TokenType::LParen)?;
        self.consume_whitespace_comments();

        // let term = self.parse_declaration()?;
        let term = self.parse_condition(FeatureKind::Supports)?;

        if !self.tokenizer.eof() {
            self.consume(TokenType::RParen)?;
        }

        Ok(Node::new(NodeType::SupportsDeclaration { term }, loc))
    }

    pub fn parse_at_rule_supports_prelude(&mut self) -> Result<Node, Error> {
        log::trace!("parse_at_rule_supports");

        self.parse_condition(FeatureKind::Supports)
    }
}
