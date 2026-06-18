use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssError;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    pub fn parse_property_name(&mut self) -> CssResult<String> {
        log::trace!("parse_property_name");
        let t = self.consume_any()?;
        match t.token_type {
            TokenType::Delim('*' | '$' | '+' | '#' | '&') => {} //next
            TokenType::Delim('/') => {
                let t = self.tokenizer.lookahead(1);
                if t.token_type == TokenType::Delim('/') {
                    self.consume_any()?;
                }
            }
            _ => {
                self.tokenizer.reconsume();
            }
        }

        let t = self.consume_any()?;
        match t.token_type {
            TokenType::Ident(value) => Ok(value),
            TokenType::Hash(value) => Ok(value),
            _ => Err(CssError::with_location(
                format!("Unexpected token {t:?}").as_str(),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn parse_declaration(&mut self) -> CssResult<Option<Node>> {
        log::trace!("parse_declaration");

        let result = self.parse_declaration_internal();
        if result.is_err() && self.config.ignore_errors {
            log::warn!("Ignoring error in parse_declaration: {result:?}");
            self.parse_until_declaration_end();
            return Ok(None);
        }

        if let Ok(declaration) = result {
            return Ok(Some(declaration));
        }
        Ok(None)
    }

    fn parse_declaration_internal(&mut self) -> CssResult<Node> {
        let loc = self.tokenizer.current_location();

        let mut important = false;

        let property = self.consume_any_ident()?;

        let custom_property = property.starts_with("--");

        self.consume_whitespace_comments();
        self.consume(TokenType::Colon)?;
        if !custom_property {
            self.consume_whitespace_comments();
        }

        self.consume_whitespace_comments();
        let value = self.parse_value_sequence()?;

        // Custom properties (`--foo`) accept an arbitrary token stream (CSS Custom Properties
        // spec), including an empty value (`--foo: ;`) and tokens the value parser does not
        // recognise (e.g. a stray `$` from unprocessed preprocessor output). Keep whatever the
        // value parser understood and skip the remainder up to the declaration's terminator,
        // rather than erroring. Regular properties still require a parseable value.
        if custom_property {
            self.skip_custom_property_remainder();
        } else if value.is_empty() {
            return Err(CssError::with_location(
                "Expected value in declaration",
                self.tokenizer.current_location(),
            ));
        }

        let t = self.consume_any()?;
        if t.is_delim('!') {
            self.consume_ident("important")?;
            self.consume_whitespace_comments();

            important = true;
        } else {
            self.tokenizer.reconsume();
        }

        Ok(Node::new(
            NodeType::Declaration {
                property,
                value,
                important,
            },
            loc,
        ))
    }

    /// Consumes any leftover custom-property value tokens up to — but not including — the
    /// declaration's terminating top-level `;` or `}`. Brackets, parentheses and `{}` blocks
    /// nested within the value are skipped over so a terminator inside them is not mistaken
    /// for the end of the declaration.
    fn skip_custom_property_remainder(&mut self) {
        let mut depth: usize = 0;
        loop {
            let t = self.tokenizer.lookahead(0);
            match t.token_type {
                TokenType::Eof => break,
                TokenType::Semicolon | TokenType::RCurly if depth == 0 => break,
                TokenType::LParen | TokenType::LBracket | TokenType::LCurly | TokenType::Function(_) => {
                    depth += 1;
                }
                TokenType::RParen | TokenType::RBracket | TokenType::RCurly => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            self.tokenizer.consume();
        }
    }

    fn parse_until_declaration_end(&mut self) {
        log::trace!(
            "parse_until_declaration_end, now at: {:?}",
            self.tokenizer.current_location()
        );
        while let Ok(t) = self.consume_any() {
            match t.token_type {
                TokenType::Semicolon => {
                    self.tokenizer.reconsume();
                    break;
                }
                TokenType::RCurly => {
                    self.tokenizer.reconsume();
                    break;
                }
                TokenType::Eof => {
                    break;
                }
                _ => {
                    // ignore
                }
            }
        }
    }
}
