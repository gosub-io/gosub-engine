use crate::node::{FeatureKind, Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use cow_utils::CowUtils;
use gosub_shared::errors::{CssError, CssResult};

impl Css3<'_> {
    fn parse_media_read_term(&mut self) -> CssResult<Node> {
        self.consume_whitespace_comments();

        let loc = self.tokenizer.current_location();

        let t = self.consume_any()?;
        match t.token_type {
            TokenType::Ident(ident) => Ok(Node::new(NodeType::Ident { value: ident }, loc)),
            TokenType::Number(value, kind) => Ok(Node::new(NodeType::Number { value, kind }, loc)),
            TokenType::Dimension { value, unit } => Ok(Node::new(NodeType::Dimension { value, unit }, loc)),
            TokenType::Function(_) => {
                self.tokenizer.reconsume(t);
                Ok(self.parse_function()?)
            }
            _ => Err(CssError::with_location(
                "Expected identifier, number, dimension, or ratio",
                loc,
            )),
        }
    }

    fn parse_media_read_comparison(&mut self) -> CssResult<Node> {
        self.consume_whitespace_comments();

        let loc = self.tokenizer.current_location();

        let delim = self.consume_any_delim()?;
        if delim == '=' {
            return Ok(Node::new(NodeType::operator("="), loc));
        }

        if delim == '>' || delim == '<' {
            // Optional `=` may be separated by whitespace, e.g. `>= 0` or `> 0`
            self.consume_whitespace_comments();
            let la = self.tokenizer.lookahead(0);
            if la.is_delim('=') {
                self.consume_any()?;
                return Ok(Node::new(NodeType::operator(format!("{delim}=")), loc));
            }
            return Ok(Node::new(NodeType::operator(format!("{delim}")), loc));
        }

        Err(CssError::with_location("Expected comparison operator", loc))
    }

    /// A comma-separated `<media-query-list>`.
    ///
    /// A query that does not parse turns into `not all` on its own, and the list goes on with
    /// the next one (Media Queries 4 §3.2): `@media junk, screen` still applies on screen. Each
    /// query is fenced in at the comma, `{` or `;` that ends it, so a failed one cannot read
    /// into the next query or the rule's block. An empty list (`@media {`) stays empty, which
    /// matches everything.
    pub fn parse_media_query_list(&mut self) -> CssResult<Node> {
        log::trace!("parse_media_query_list");

        let loc = self.tokenizer.current_location();

        let mut queries = vec![];

        loop {
            self.consume_whitespace_comments();
            let len = self.media_query_extent();
            let boundary = self.tokenizer.fence(len);
            let is_comma = boundary.is_comma();
            if len == 0 && !is_comma && queries.is_empty() {
                self.tokenizer.consume();
                self.tokenizer.reconsume(boundary);
                break;
            }

            let query_loc = self.tokenizer.current_location();
            let start = self.tokenizer.consumed();
            let query = self.parse_media_query();
            if self.tokenizer.consumed() - start <= len {
                // Only while the fence is still ahead: past it lies the next query.
                self.consume_whitespace_comments();
            }
            let used = self.tokenizer.consumed() - start;
            if used > len + 1 {
                // Past the fence: the tokens the query read beyond it are gone.
                return Err(CssError::with_location("media query ran past its end", query_loc));
            }
            let complete = used == len;
            for _ in used..=len {
                self.tokenizer.consume();
            }
            self.tokenizer.reconsume(boundary);

            queries.push(match query {
                Ok(query) if complete => query,
                _ => {
                    log::debug!("unreadable media query at {query_loc:?}: not all");
                    Node::new(
                        NodeType::MediaQuery {
                            modifier: "not".into(),
                            media_type: "all".into(),
                            condition: None,
                        },
                        query_loc,
                    )
                }
            });

            if !is_comma {
                break;
            }
            self.tokenizer.consume();
        }

        Ok(Node::new(NodeType::MediaQueryList { media_queries: queries }, loc))
    }

    /// How many tokens the next media query spans: everything up to a comma, `{` or `;` outside
    /// any block, a closing bracket of a block opened before the query, or the end of input.
    ///
    /// A block ends only at its own closing bracket, so in `print and (], screen` the comma is
    /// still inside the `(` and `screen` is not a query of its own.
    fn media_query_extent(&mut self) -> usize {
        let mut open: Vec<TokenType> = Vec::new();
        let mut len = 0;
        loop {
            let token_type = &self.tokenizer.lookahead(len).token_type;
            match token_type {
                TokenType::Eof => return len,
                TokenType::Comma | TokenType::LCurly | TokenType::Semicolon if open.is_empty() => return len,
                TokenType::RParen | TokenType::RBracket | TokenType::RCurly if open.is_empty() => return len,
                TokenType::LParen | TokenType::Function(_) => open.push(TokenType::RParen),
                TokenType::LBracket => open.push(TokenType::RBracket),
                TokenType::LCurly => open.push(TokenType::RCurly),
                // A closing bracket of another kind is just a token inside the block.
                TokenType::RParen | TokenType::RBracket | TokenType::RCurly if open.last() == Some(token_type) => {
                    open.pop();
                }
                _ => {}
            }
            len += 1;
        }
    }

    fn parse_media_feature_feature(&mut self, kind: FeatureKind) -> CssResult<Node> {
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
                return Err(CssError::with_location("Expected colon", t.location));
            }
            self.consume_whitespace_comments();

            let t = self.consume_any()?;
            let first = match t.token_type {
                TokenType::Number(value, kind) => Node::new(NodeType::Number { value, kind }, t.location),
                TokenType::Dimension { value, unit } => Node::new(NodeType::Dimension { value, unit }, t.location),
                TokenType::Ident(value) => Node::new(NodeType::Ident { value }, t.location),
                TokenType::Function(_) => {
                    self.tokenizer.reconsume(t);
                    self.parse_function()?
                }
                _ => {
                    return Err(CssError::with_location(
                        "Expected identifier, number, dimension, or ratio",
                        t.location,
                    ));
                }
            };

            self.consume_whitespace_comments();

            // Optional `<ratio>` (`<number> / <number>`), e.g. `aspect-ratio: 16/9`.
            value = if self.tokenizer.lookahead(0).is_delim('/') {
                let op_loc = self.tokenizer.current_location();
                self.consume_any()?;
                let second = self.parse_media_read_term()?;
                Some(Node::new(
                    NodeType::Value {
                        children: vec![first, Node::new(NodeType::operator("/"), op_loc), second],
                    },
                    loc,
                ))
            } else {
                Some(first)
            };

            self.consume_whitespace_comments();

            if !self.tokenizer.eof() {
                self.consume(TokenType::RParen)?;
            }
        }

        Ok(Node::new(
            NodeType::Feature {
                kind,
                name,
                value: value.map(Box::new),
            },
            loc,
        ))
    }

    fn parse_media_feature_range(&mut self, _kind: FeatureKind) -> CssResult<Node> {
        log::trace!("parse_media_feature_range");

        let loc = self.tokenizer.current_location();

        self.consume_whitespace_comments();
        self.consume(TokenType::LParen)?;

        let left = self.parse_media_read_term()?;
        let left_comparison = self.parse_media_read_comparison()?;
        let middle = self.parse_media_read_term()?;
        let mut right_comparison = None;
        let mut right = None;

        // Optional second comparison for a double-ended range, e.g. `400px <= width <= 700px`.
        // It is introduced by another comparison operator (`<`, `>` or `=`), not a parenthesis.
        self.consume_whitespace_comments();
        if matches!(
            self.tokenizer.lookahead_sc(0).token_type,
            TokenType::Delim('<' | '>' | '=')
        ) {
            right_comparison = Some(self.parse_media_read_comparison()?);
            right = Some(self.parse_media_read_term()?);
        }

        self.consume_whitespace_comments();
        // Parentheses are tokenized as `RParen`, not `Delim(')')`.
        self.consume(TokenType::RParen)?;

        Ok(Node::new(
            NodeType::Range {
                left: Box::new(left),
                left_comparison: Box::new(left_comparison),
                middle: Box::new(middle),
                right_comparison: right_comparison.map(Box::new),
                right: right.map(Box::new),
            },
            loc,
        ))
    }

    pub fn parse_media_feature_or_range(&mut self, kind: FeatureKind) -> CssResult<Node> {
        log::trace!("parse_media_feature_or_range");

        let t_is_ident = self.tokenizer.lookahead_sc(1).is_ident();
        let nt = self.tokenizer.lookahead_sc(2);
        if t_is_ident && (nt.is_colon() || nt.token_type == TokenType::RParen) {
            // feature
            return self.parse_media_feature_feature(kind);
        }

        // otherwise it's a range
        self.parse_media_feature_range(kind)
    }

    pub fn parse_media_query(&mut self) -> CssResult<Node> {
        log::trace!("parse_media_query");

        let loc = self.tokenizer.current_location();

        let mut modifier = String::new();
        let mut media_type = String::new();
        let mut condition = None;

        self.consume_whitespace_comments();
        let t = self.consume_any()?;

        // Asking once for the identifier, rather than testing `is_ident()` and then matching
        // for it again with an `unreachable!()` for the case the test already excluded. `t`
        // itself is kept whole, because the `else` branch below reconsumes it.
        let nt = self.tokenizer.lookahead_sc(0);
        let bare_ident = match &t.token_type {
            TokenType::Ident(ident) if nt.token_type != TokenType::LParen => Some(ident.clone()),
            _ => None,
        };

        if let Some(ident) = bare_ident {
            let s = ident.cow_to_lowercase();
            media_type = if ["not", "only"].contains(&s.as_ref()) {
                self.consume_whitespace_comments();
                modifier = ident;
                self.consume_any_ident()?
            } else {
                ident
            };

            self.consume_whitespace_comments();
            let nt = self.tokenizer.lookahead_sc(0);
            match &nt.token_type {
                TokenType::Ident(s) => {
                    if s.as_str() != "and" {
                        return Err(CssError::with_location("Expected 'and'", t.location));
                    }

                    self.consume_ident("and")?;
                    condition = Some(self.parse_condition(FeatureKind::Media)?);
                }
                // End of input too: a list read from an attribute (`<source media="print">`)
                // has no block or semicolon after it.
                TokenType::LCurly | TokenType::Semicolon | TokenType::Comma | TokenType::Eof => {
                    // skip;
                }
                _ => {
                    return Err(CssError::with_location(
                        "Expected identifier or parenthesis",
                        t.location,
                    ));
                }
            }
        } else {
            //
            match t.token_type {
                TokenType::Ident(_) | TokenType::LParen | TokenType::Function(_) => {
                    self.tokenizer.reconsume(t);
                    condition = Some(self.parse_condition(FeatureKind::Media)?);
                }
                TokenType::LCurly | TokenType::Semicolon => {
                    // skip
                }
                _ => {
                    return Err(CssError::with_location(
                        "Expected identifier or parenthesis",
                        t.location,
                    ));
                }
            }
        }

        Ok(Node::new(
            NodeType::MediaQuery {
                modifier,
                media_type,
                condition: condition.map(Box::new),
            },
            loc,
        ))
    }

    pub fn parse_at_rule_media_prelude(&mut self) -> CssResult<Node> {
        log::trace!("parse_at_rule_media_prelude");

        self.parse_media_query_list()
    }

    /// Parses the prelude of a `@custom-media` rule (Media Queries Level 5 / PostCSS):
    /// `@custom-media <extension-name> <media-query-list>;`, e.g.
    /// `@custom-media --screen-max-xxl (max-width: 1440px);`. The value can also be the
    /// keywords `true`/`false`, which the media-query parser accepts as a media type.
    pub fn parse_at_rule_custom_media_prelude(&mut self) -> CssResult<Node> {
        log::trace!("parse_at_rule_custom_media_prelude");

        let loc = self.tokenizer.current_location();

        let name = self.consume_any_ident()?;
        self.consume_whitespace_comments();

        let query_list = self.parse_media_query_list()?;

        Ok(Node::new(
            NodeType::Value {
                children: vec![Node::new(NodeType::Ident { value: name }, loc), query_list],
            },
            loc,
        ))
    }
}
