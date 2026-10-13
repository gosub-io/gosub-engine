use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssError;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    fn parse_pseudo_function_selector_list(&mut self) -> CssResult<Node> {
        log::trace!("parse_pseudo_function_selector_list");
        self.parse_selector_list()
    }

    /// `:is()` and `:where()` take a forgiving selector list (Selectors L4): an argument that
    /// does not parse is dropped on its own, and the others still count. The extent of each
    /// argument is found first, up to the next top-level `,` or `)`, so a failed attempt can skip
    /// what it left of it. An attempt that took the `,` or `)` ending the argument (a parse
    /// function that fails on the token after it) gets that token back; one that ran further is
    /// an error, as before.
    fn parse_forgiving_selector_list(&mut self) -> CssResult<Node> {
        log::trace!("parse_forgiving_selector_list");

        let loc = self.tokenizer.current_location();
        let mut selectors = vec![];

        loop {
            // A space after `(` or `,` is not a descendant combinator.
            self.consume_whitespace_comments();
            let extent = self.argument_extent();
            let boundary = self.tokenizer.lookahead(extent).clone();
            let start = self.tokenizer.consumed();
            let parsed = self.parse_complex_selector();
            if parsed.is_ok() {
                self.consume_whitespace_comments();
            }
            let used = self.tokenizer.consumed() - start;
            let at_end = matches!(
                self.tokenizer.lookahead(0).token_type,
                TokenType::Comma | TokenType::RParen | TokenType::Eof
            );

            match parsed {
                Ok(selector) if at_end => selectors.push(selector),
                _ if used <= extent => {
                    for _ in used..extent {
                        self.tokenizer.consume();
                    }
                }
                _ if used == extent + 1 => self.tokenizer.reconsume(boundary),
                Err(err) => return Err(err),
                Ok(_) => {
                    return Err(CssError::with_location(
                        "selector argument ran past its end",
                        self.tokenizer.current_location(),
                    ))
                }
            }

            if !matches!(self.tokenizer.lookahead(0).token_type, TokenType::Comma) {
                break;
            }
            self.tokenizer.consume();
        }

        Ok(Node::new(NodeType::SelectorList { selectors }, loc))
    }

    /// The number of tokens up to the next `,` or `)` outside any bracket, without consuming them.
    fn argument_extent(&mut self) -> usize {
        let mut depth: usize = 0;
        let mut i = 0;
        loop {
            match self.tokenizer.lookahead(i).token_type {
                TokenType::Eof => return i,
                TokenType::Comma | TokenType::RParen | TokenType::RCurly if depth == 0 => return i,
                TokenType::LParen | TokenType::LBracket | TokenType::LCurly | TokenType::Function(_) => depth += 1,
                TokenType::RParen | TokenType::RBracket | TokenType::RCurly => depth = depth.saturating_sub(1),
                _ => {}
            }
            i += 1;
        }
    }

    fn parse_pseudo_function_selector(&mut self) -> CssResult<Node> {
        log::trace!("parse_pseudo_function_selector");

        self.parse_selector()
    }

    fn parse_pseudo_function_ident_list(&mut self) -> CssResult<Node> {
        log::trace!("parse_pseudo_function_ident_list");

        let loc = self.tokenizer.current_location();

        let value = self.consume_any_ident()?;

        Ok(Node::new(NodeType::Ident { value }, loc))
    }

    /// Reads the language ranges of `:lang()`: a comma-separated list of idents or strings
    /// (Selectors L4 §7.2), e.g. `:lang(en, "de-*")`.
    fn parse_pseudo_function_lang_ranges(&mut self) -> CssResult<Node> {
        log::trace!("parse_pseudo_function_lang_ranges");

        let loc = self.tokenizer.current_location();

        let mut children = Vec::new();
        loop {
            self.consume_whitespace_comments();
            let t = self.tokenizer.consume();
            match t.token_type {
                TokenType::Ident(value) => children.push(Node::new(NodeType::Ident { value }, t.location)),
                TokenType::QuotedString(value) => children.push(Node::new(NodeType::String { value }, t.location)),
                _ => {
                    return Err(CssError::with_location(
                        format!("Expected a language range, got {t:?}").as_str(),
                        t.location,
                    ));
                }
            }
            self.consume_whitespace_comments();
            if matches!(self.tokenizer.lookahead(0).token_type, TokenType::Comma) {
                self.tokenizer.consume();
            } else {
                break;
            }
        }

        Ok(Node::new(NodeType::Value { children }, loc))
    }

    /// Reads a whitespace-separated sequence of idents, e.g. the part name list of `::part(a b)`.
    fn parse_pseudo_function_ident_sequence(&mut self) -> CssResult<Node> {
        log::trace!("parse_pseudo_function_ident_sequence");

        let loc = self.tokenizer.current_location();

        let mut children = Vec::new();
        loop {
            self.consume_whitespace_comments();
            if let TokenType::Ident(_) = self.tokenizer.lookahead(0).token_type {
                let value = self.consume_any_ident()?;
                children.push(Node::new(NodeType::Ident { value }, loc));
            } else {
                break;
            }
        }

        Ok(Node::new(NodeType::Value { children }, loc))
    }

    fn parse_pseudo_function_nth(&mut self) -> CssResult<Node> {
        log::trace!("parse_pseudo_function_nth");

        self.consume_whitespace_comments();

        let loc = self.tokenizer.current_location();

        let mut selector = None;

        let t = self.consume_any()?;
        let nth = match t.token_type {
            TokenType::Ident(ref value) if value.eq_ignore_ascii_case("odd") => Node::new(
                NodeType::AnPlusB {
                    a: "2".into(),
                    b: "1".into(),
                },
                loc,
            ),
            TokenType::Ident(ref value) if value.eq_ignore_ascii_case("even") => Node::new(
                NodeType::AnPlusB {
                    a: "2".into(),
                    b: "0".into(),
                },
                loc,
            ),
            TokenType::Ident(_) => {
                self.tokenizer.reconsume(t);
                self.parse_anplusb()?
            }
            TokenType::Dimension { .. } => {
                self.tokenizer.reconsume(t);
                self.parse_anplusb()?
            }
            TokenType::Number(value, kind) => Node::new(NodeType::Number { value, kind }, loc),
            _ => {
                return Err(CssError::with_location(
                    format!("Unexpected token {:?}", self.tokenizer.lookahead(0)).as_str(),
                    self.tokenizer.current_location(),
                ));
            }
        };

        self.consume_whitespace_comments();

        let is_of = matches!(&self.tokenizer.lookahead(0).token_type, TokenType::Ident(value) if value.eq_ignore_ascii_case("of"));
        // Any other identifier is left for the caller's `)` check to reject.
        if is_of {
            self.consume_any()?;
            // The space after `of` separates it from the list; left in, it would parse as a
            // leading descendant combinator.
            self.consume_whitespace_comments();
            selector = Some(self.parse_selector_list()?);
        }

        Ok(Node::new(
            NodeType::Nth {
                nth: Box::new(nth),
                selector: selector.map(Box::new),
            },
            loc,
        ))
    }

    /// `:is(...)`, `:not(...)` and friends take a selector list, which may contain further pseudo
    /// functions, so the body parses one recursion level deeper.
    pub(crate) fn parse_pseudo_function(&mut self, name: &str) -> CssResult<Node> {
        self.recurse(|parser| parser.parse_pseudo_function_inner(name))
    }

    fn parse_pseudo_function_inner(&mut self, name: &str) -> CssResult<Node> {
        log::trace!("parse_pseudo_function");
        match name {
            "dir" => self.parse_pseudo_function_ident_list(),
            "has" => self.parse_pseudo_function_selector_list(),
            "lang" => self.parse_pseudo_function_lang_ranges(),
            "matches" => self.parse_forgiving_selector_list(),
            "is" => self.parse_forgiving_selector_list(),
            "-moz-any" => self.parse_pseudo_function_selector_list(),
            "-webkit-any" => self.parse_pseudo_function_selector_list(),
            "where" => self.parse_forgiving_selector_list(),
            "not" => self.parse_pseudo_function_selector_list(),
            "nth-child" => self.parse_pseudo_function_nth(),
            "nth-last-child" => self.parse_pseudo_function_nth(),
            "nth-last-of-type" => self.parse_pseudo_function_nth(),
            "nth-of-type" => self.parse_pseudo_function_nth(),
            "slotted" => self.parse_pseudo_function_selector(),
            "host" => self.parse_pseudo_function_selector(),
            "host-context" => self.parse_pseudo_function_selector(),
            "part" => self.parse_pseudo_function_ident_sequence(),
            "highlight" => self.parse_pseudo_function_ident_list(),
            _ => Err(CssError::with_location(
                format!("Unexpected pseudo function {name:?}").as_str(),
                self.tokenizer.current_location(),
            )),
        }
    }
}
