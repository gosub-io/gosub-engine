use crate::css3::node::{FeatureKind, Node, NodeType};
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    fn parse_media_feature_feature(&mut self, kind: FeatureKind) -> Result<Node, Error> {
        log::trace!("parse_media_feature_feature");

        let loc = self.tokenizer.current_location().clone();

        self.consume(TokenType::LParen)?;

        let mut value: Option<Node> = None;

        self.consume_whitespace_comments();

        let name = self.consume_any_ident()?;
        self.consume_whitespace_comments();

        let t = self.consume_any()?;
        self.consume_whitespace_comments();

        if t.token_type != TokenType::RParen {
            if !t.is_colon() {
                return Err(Error::new("Expected colon".to_string(), t.location));
            }
            self.consume_whitespace_comments();

            let t = self.consume_any()?;
            value = match t.token_type {
                TokenType::Number(value) => Some(Node::new(NodeType::Number { value }, t.location)),
                TokenType::Dimension { value, unit } => {
                    Some(Node::new(NodeType::Dimension { value, unit }, t.location))
                }
                TokenType::Ident(value) => Some(Node::new(NodeType::Ident { value }, t.location)),
                TokenType::Function(name) => {
                    let name = name.to_lowercase();
                    let args = self.parse_pseudo_function(name.as_str())?;
                    self.consume(TokenType::RParen)?;

                    Some(Node::new(NodeType::Function { name, arguments: vec![args] }, t.location))
                }
                _ => {
                    return Err(Error::new(
                        "Expected identifier, number, dimension, or ratio".to_string(),
                        t.location,
                    ));
                }
            };

            self.consume_whitespace_comments();

            if !self.tokenizer.eof() {
                self.consume(TokenType::RParen)?;
            }
        }

        Ok(Node::new(NodeType::Feature { kind, name, value }, loc))
    }

    fn parse_media_feature_range(&mut self, _kind: FeatureKind) -> Result<Node, Error> {
        log::trace!("parse_media_feature_range");

        todo!();
        // Ok(Node::new(NodeType::Ident{value: "foo".into()}))
    }

    pub fn parse_media_feature_or_range(&mut self, kind: FeatureKind) -> Result<Node, Error> {
        log::trace!("parse_media_feature_or_range");

        let t = self.tokenizer.lookahead_sc(1);
        let nt = self.tokenizer.lookahead_sc(2);
        if t.is_ident()
            && (nt.is_colon()
                || nt.token_type == TokenType::RParen)
        {
            // feature
            return self.parse_media_feature_feature(kind);
        }

        // otherwise it's a range
        self.parse_media_feature_range(kind)
    }

    pub fn parse_media_query(&mut self) -> Result<Node, Error> {
        log::trace!("parse_media_query");

        let loc = self.tokenizer.current_location().clone();

        let mut modifier = "".into();
        let mut media_type = "".into();
        let mut condition = None;

        self.consume_whitespace_comments();
        let t = self.consume_any()?;

        let nt = self.tokenizer.lookahead_sc(0);
        if t.is_ident() && nt.token_type != TokenType::LParen {
            let ident = match t.token_type {
                TokenType::Ident(s) => s,
                _ => unreachable!(),
            };

            let s = ident.to_lowercase();
            media_type = if ["not", "only"].contains(&s.as_str()) {
                self.consume_whitespace_comments();
                modifier = ident;
                self.consume_any_ident()?
            } else {
                ident
            };

            self.consume_whitespace_comments();
            let nt = self.tokenizer.lookahead_sc(0);
            match nt.token_type {
                TokenType::Ident(s) => {
                    if s != "and" {
                        return Err(Error::new("Expected 'and'".to_string(), nt.location));
                    }

                    self.consume_ident("and")?;
                    condition = Some(self.parse_condition(FeatureKind::Media)?);
                }
                TokenType::LCurly | TokenType::Semicolon | TokenType::Comma => {
                    // skip;
                }
                _ => {
                    return Err(Error::new(
                        "Expected identifier or parenthesis".to_string(),
                        t.location,
                    ));
                }
            }
        } else {
            //
            match t.token_type {
                TokenType::Ident(_) | TokenType::LParen | TokenType::Function(_) => {
                    self.tokenizer.reconsume();
                    condition = Some(self.parse_condition(FeatureKind::Media)?);
                }
                TokenType::LCurly | TokenType::Semicolon => {
                    // skip
                }
                _ => {
                    return Err(Error::new(
                        "Expected identifier or parenthesis".to_string(),
                        t.location,
                    ));
                }
            }
        }

        Ok(Node::new(NodeType::MediaQuery { modifier, media_type, condition }, loc))
    }

    pub fn parse_at_rule_media_prelude(&mut self) -> Result<Node, Error> {
        log::trace!("parse_at_rule_media");

        self.parse_at_rule_prelude_query_list()
    }
}
