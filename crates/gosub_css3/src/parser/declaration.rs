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
            // `!important` allows trivia after the `!` and is case-insensitive.
            self.consume_whitespace_comments();
            let ident = self.consume_any_ident()?;
            if !ident.eq_ignore_ascii_case("important") {
                return Err(CssError::with_location(
                    format!("Expected important, got {ident}").as_str(),
                    self.tokenizer.current_location(),
                ));
            }
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

    /// Consumes any leftover custom-property value tokens up to - but not including - the
    /// declaration's terminating top-level `;` or `}`. Brackets, parentheses and `{}` blocks
    /// nested within the value are skipped over so a terminator inside them is not mistaken
    /// for the end of the declaration.
    /// Skip the rest of a custom property's token stream up to its terminator. A trailing
    /// `!important` (the last two significant tokens) is not part of the value: it is left in
    /// place for the importance check, like on any other declaration. A `!` anywhere else
    /// (`--x: a !foo`) is just another token of the arbitrary stream.
    fn skip_custom_property_remainder(&mut self) {
        // Find the terminator and, on the way, where a trailing `!important` would start.
        let mut depth: usize = 0;
        let mut end = 0;
        let mut last_two: [Option<usize>; 2] = [None, None];
        loop {
            let t = self.tokenizer.lookahead(end);
            match t.token_type {
                TokenType::Eof => break,
                TokenType::Semicolon | TokenType::RCurly if depth == 0 => break,
                TokenType::LParen | TokenType::LBracket | TokenType::LCurly | TokenType::Function(_) => {
                    depth += 1;
                }
                TokenType::RParen | TokenType::RBracket | TokenType::RCurly => {
                    depth = saturating_dec(depth);
                }
                _ => {}
            }
            if !matches!(t.token_type, TokenType::Whitespace(_) | TokenType::Comment(_)) {
                last_two = [last_two[1], Some(end)];
            }
            end += 1;
        }

        // Same shape the importance check accepts: `!`, optional trivia, `important` in any case.
        let stop = match last_two {
            [Some(bang), Some(ident)]
                if self.tokenizer.lookahead(bang).is_delim('!')
                    && matches!(&self.tokenizer.lookahead(ident).token_type,
                        TokenType::Ident(name) if name.eq_ignore_ascii_case("important")) =>
            {
                bang
            }
            _ => end,
        };
        for _ in 0..stop {
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

fn saturating_dec(depth: usize) -> usize {
    depth.saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use crate::stylesheet::CssValue;
    use crate::{Css3, ParserConfig};
    use gosub_interface::css3::CssOrigin;

    fn declarations(css: &str) -> Vec<(String, CssValue, bool)> {
        let sheet = Css3::parse_str(css, ParserConfig::default(), CssOrigin::Author, "test").expect("parse");
        sheet.rules[0]
            .declarations()
            .iter()
            .map(|d| (d.property.clone(), d.value.clone(), d.important))
            .collect()
    }

    #[test]
    fn custom_property_trailing_important_is_recognized() {
        let decls = declarations("div { --w: 200px !important; width: 1px }");
        assert_eq!(decls.len(), 2, "{decls:?}");
        assert_eq!(decls[0].0, "--w");
        assert!(decls[0].2, "custom property must carry !important: {decls:?}");
        assert!(!decls[1].2);

        // Trivia after `!` and any letter case are valid, on custom and regular properties.
        let decls = declarations(
            "div { --a: 1 ! important; --b: 2 !/**/IMPORTANT; --c: 3 !Important; width: 1px ! Important }",
        );
        assert_eq!(decls.len(), 4, "{decls:?}");
        assert!(decls.iter().all(|d| d.2), "{decls:?}");
    }

    #[test]
    fn custom_property_keeps_arbitrary_bang_tokens() {
        // `!foo` is part of the token stream, not an importance flag, and must not reject the
        // declaration or the ones after it.
        let decls = declarations("div { --x: value !foo; --y: a ! b !important c; --z: q !nope; width: 1px }");
        assert_eq!(decls.len(), 4, "{decls:?}");
        assert_eq!(decls[0].0, "--x");
        assert!(!decls[0].2);
        assert_eq!(decls[1].0, "--y");
        assert!(!decls[1].2);
        assert_eq!(decls[2].0, "--z");
        assert!(!decls[2].2);
        assert_eq!(decls[3].0, "width");
    }
}
