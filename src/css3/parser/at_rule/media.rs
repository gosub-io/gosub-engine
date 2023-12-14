use crate::css3::node::{FeatureKind, Node, NodeType};
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    fn parse_media_feature_feature(&mut self, kind: FeatureKind) -> Result<Node, Error> {
        log::trace!("parse_media_feature_feature");

        let loc = self.tokenizer.current_location().clone();

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

            let t = self.consume_any()?;
            value = match t.token_type {
                TokenType::Number(value) => Some(Node::new(NodeType::Number { value }, t.location)),
                TokenType::Dimension { value, unit } => {
                    Some(Node::new(NodeType::Dimension { value, unit }, t.location))
                }
                TokenType::Ident(value) => Some(Node::new(NodeType::Ident { value }, t.location)),
                TokenType::Function(_) => {
                    todo!();
                    // Some(t)
                }
                _ => {
                    return Err(Error::new(
                        "Expected identifier, number, dimension, or ratio".to_string(),
                        t.location,
                    ));
                }
            };

            self.consume_whitespace_comments();
        }

        Ok(Node::new(NodeType::Feature { kind, name, value }, loc))
    }

    fn parse_media_feature_range(&mut self, _kind: FeatureKind) -> Result<Node, Error> {
        log::trace!("parse_media_feature_range");

        todo!();
        // Ok(Node::new(NodeType::Ident{value: "foo".into()}))
    }

    fn parse_media_feature_or_range(&mut self, kind: FeatureKind) -> Result<Node, Error> {
        log::trace!("parse_media_feature_or_range");

        let t = self.tokenizer.lookahead_sc(0);
        let nt = self.tokenizer.lookahead_sc(1);
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

    pub fn parse_media_condition(&mut self, kind: FeatureKind) -> Result<Node, Error> {
        log::trace!("parse_media_condition");

        let loc = self.tokenizer.current_location().clone();

        let mut list = Vec::new();

        loop {
            let t = self.consume_any()?;
            match t.token_type {
                TokenType::Comment(_) | TokenType::Whitespace => {
                    // skip
                    continue;
                }
                TokenType::Ident(ident) => {
                    list.push(Node::new(NodeType::Ident { value: ident }, t.location));
                }
                TokenType::LParen => match kind {
                    FeatureKind::Media => {
                        list.push(self.parse_media_feature_or_range(kind.clone())?);
                        self.consume(TokenType::RParen)?;
                        break;
                    }
                    FeatureKind::Container => {
                        list.push(self.parse_media_feature_or_range(kind.clone())?);
                        self.consume(TokenType::RParen)?;
                        break;
                    }
                },
                TokenType::Function(_) => {
                    todo!();
                }
                _ => {
                    break;
                }
            }
        }

        Ok(Node::new(NodeType::Condition { list }, loc))
    }

    pub fn parse_media_query(&mut self) -> Result<Node, Error> {
        log::trace!("parse_media_query");

        let loc = self.tokenizer.current_location().clone();

        let mut modifier = "".into();
        let mut media_type = "".into();
        let mut condition = None;

        self.consume_whitespace_comments();
        let t = self.consume_any()?;

        self.consume_whitespace_comments();
        let nt = self.tokenizer.lookahead(0);
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
            let nt = self.tokenizer.lookahead(0);
            match nt.token_type {
                TokenType::Ident(s) => {
                    if s != "and" {
                        return Err(Error::new("Expected 'and'".to_string(), nt.location));
                    }

                    self.consume_ident("and")?;
                    condition = Some(self.parse_media_condition(FeatureKind::Media)?);
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
                    condition = Some(self.parse_media_condition(FeatureKind::Media)?);
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

    pub fn parse_media_query_list(&mut self) -> Result<Node, Error> {
        log::trace!("parse_media_query_list");

        let loc = self.tokenizer.current_location().clone();

        let mut queries = vec![];

        while !self.tokenizer.eof() {
            let query = self.parse_media_query()?;
            queries.push(query);

            self.consume_whitespace_comments();

            let t = self.consume_any()?;
            if !t.is_comma() {
                self.tokenizer.reconsume();
                break;
            }
        }

        Ok(Node::new(NodeType::MediaQueryList { media_queries: queries }, loc))
    }

    pub fn parse_at_rule_media(&mut self) -> Result<Node, Error> {
        log::trace!("parse_at_rule_media");

        let node = self.parse_media_query_list()?;

        Ok(node)
    }
}
