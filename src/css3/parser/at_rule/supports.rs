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

    fn parse_at_rule_supports_condition(&mut self) -> Result<Node, Error> {
        loop {
            let t = self.consume_any()?;
            match t.token_type {
                TokenType::Ident(ident) if ident.eq_ignore_ascii_case("not") => {
                    let term = self.parse_supports_condition_parens()?;
                    return Ok(Node::new(NodeType::SupportsNot { term }, t.location));
                }
            }
        }
    }

    pub fn parse_at_rule_supports_prelude(&mut self) -> Result<Node, Error> {
        log::trace!("parse_at_rule_supports_prelude");

        loop {
            let t = self.consume_any()?;
        }

        // not

        // optional (
        <supports condition>
        // and / or

        self.parse_condition(FeatureKind::Supports)
    }
}
