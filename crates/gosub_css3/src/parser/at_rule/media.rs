use crate::node::{FeatureKind, Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    fn parse_media_read_term(&mut self) -> Result<Node, Error> {
        self.consume_whitespace_comments();

        let loc = self.tokenizer.current_location();

        let t = self.consume_any()?;
        match t.token_type {
            TokenType::Ident(ident) => Ok(Node::new(NodeType::Ident { value: ident }, loc)),
            TokenType::Number(value) => Ok(Node::new(NodeType::Number { value }, loc)),
            TokenType::Dimension { value, unit } => {
                Ok(Node::new(NodeType::Dimension { value, unit }, loc))
            }
            TokenType::Function(name) => {
                let name = name.to_lowercase();
                let args = self.parse_pseudo_function(name.as_str())?;
                self.consume(TokenType::RParen)?;

                Ok(Node::new(
                    NodeType::Function {
                        name,
                        arguments: vec![args],
                    },
                    loc,
                ))
            }
            _ => Err(Error::new(
                "Expected identifier, number, dimension, or ratio".to_string(),
                loc,
            )),
        }
    }

    fn parse_media_read_comparison(&mut self) -> Result<Node, Error> {
        self.consume_whitespace_comments();

        let loc = self.tokenizer.current_location();

        let delim = self.consume_any_delim()?;
        if delim == '=' {
            return Ok(Node::new(NodeType::Operator("=".into()), loc));
        }

        if delim == '>' || delim == '<' {
            let eq_sign = self.consume_any_delim()?;
            if eq_sign == '=' {
                return Ok(Node::new(NodeType::Operator(format!("{}=", delim)), loc));
            }

            self.tokenizer.reconsume();
            return Ok(Node::new(NodeType::Operator(format!("{}", delim)), loc));
        }

        Err(Error::new("Expected comparison operator".to_string(), loc))
    }

    pub fn parse_media_query_list(&mut self) -> Result<Node, Error> {
        log::trace!("parse_media_query_list");

        let loc = self.tokenizer.current_location();

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

        Ok(Node::new(
            NodeType::MediaQueryList {
                media_queries: queries,
            },
            loc,
        ))
    }

    fn parse_media_feature_feature(&mut self, kind: FeatureKind) -> Result<Node, Error> {
        log::trace!("parse_media_feature_feature");

        let loc = self.tokenizer.current_location();

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

                    Some(Node::new(
                        NodeType::Function {
                            name,
                            arguments: vec![args],
                        },
                        t.location,
                    ))
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

        let loc = self.tokenizer.current_location();

        self.consume_whitespace_comments();
        self.consume(TokenType::LParen)?;

        let left = self.parse_media_read_term()?;
        let left_comparison = self.parse_media_read_comparison()?;
        let middle = self.parse_media_read_term()?;
        let mut right_comparison = None;
        let mut right = None;

        if self.tokenizer.lookahead_sc(0).is_delim('(') {
            right_comparison = Some(self.parse_media_read_comparison()?);
            right = Some(self.parse_media_read_term()?);
        }

        self.consume_whitespace_comments();
        self.consume_delim(')')?;

        Ok(Node::new(
            NodeType::Range {
                left,
                left_comparison,
                middle,
                right_comparison,
                right,
            },
            loc,
        ))
    }

    pub fn parse_media_feature_or_range(&mut self, kind: FeatureKind) -> Result<Node, Error> {
        log::trace!("parse_media_feature_or_range");

        let t = self.tokenizer.lookahead_sc(1);
        let nt = self.tokenizer.lookahead_sc(2);
        if t.is_ident() && (nt.is_colon() || nt.token_type == TokenType::RParen) {
            // feature
            return self.parse_media_feature_feature(kind);
        }

        // otherwise it's a range
        self.parse_media_feature_range(kind)
    }

    pub fn parse_media_query(&mut self) -> Result<Node, Error> {
        log::trace!("parse_media_query");

        let loc = self.tokenizer.current_location();

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

        Ok(Node::new(
            NodeType::MediaQuery {
                modifier,
                media_type,
                condition,
            },
            loc,
        ))
    }

    pub fn parse_at_rule_media_prelude(&mut self) -> Result<Node, Error> {
        log::trace!("parse_at_rule_media_prelude");

        self.parse_media_query_list()
    }
}
