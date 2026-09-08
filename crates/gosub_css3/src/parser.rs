use crate::tokenizer::{Number, Token, TokenType};
use crate::Css3;
use gosub_shared::errors::{CssError, CssResult};

mod anplusb;
mod at_rule;
mod block;
mod calc;
mod combinator;
mod condition;
mod declaration;
mod feature_function;
mod function;
mod operator;
mod pseudo;
mod rule;
mod selector;
mod selector_list;
mod stylesheet;
mod url;
mod value;

impl Css3<'_> {
    /// Consumes a specific token
    pub fn consume(&mut self, token_type: TokenType) -> CssResult<Token> {
        let t = self.tokenizer.consume();
        if t.token_type != token_type {
            return Err(CssError::with_location(
                format!("Expected {token_type:?}, got {t:?}").as_str(),
                self.tokenizer.current_location(),
            ));
        }

        Ok(t)
    }

    /// Consumes any token
    pub fn consume_any(&mut self) -> CssResult<Token> {
        Ok(self.tokenizer.consume())
    }

    pub fn consume_function(&mut self) -> CssResult<String> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Function(name) => Ok(name),
            _ => Err(CssError::with_location(
                format!("Expected function, got {t:?}").as_str(),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_any_number(&mut self) -> CssResult<Number> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Number(value) => Ok(value),
            _ => Err(CssError::with_location(
                format!("Expected number, got {t:?}").as_str(),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_any_delim(&mut self) -> CssResult<char> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Delim(c) => Ok(c),
            _ => Err(CssError::with_location(
                format!("Expected delimiter, got {t:?}").as_str(),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_any_string(&mut self) -> CssResult<String> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::QuotedString(s) => Ok(s),
            _ => Err(CssError::with_location(
                format!("Expected string, got {t:?}").as_str(),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_delim(&mut self, delimiter: char) -> CssResult<char> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Delim(c) if c == delimiter => Ok(c),
            _ => Err(CssError::with_location(
                format!("Expected delimiter '{delimiter}', got {t:?}").as_str(),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_whitespace_comments(&mut self) {
        loop {
            let t = self.tokenizer.consume();
            match t.token_type {
                TokenType::Whitespace(_) | TokenType::Comment(_) => {
                    // discard and keep consuming
                }
                _ => {
                    self.tokenizer.reconsume(t);
                    break;
                }
            }
        }
    }

    pub fn consume_ident_ci(&mut self, ident: &str) -> CssResult<String> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Ident(s) if s.eq_ignore_ascii_case(ident) => Ok(s),
            _ => Err(CssError::with_location(
                format!("Expected ident, got {t:?}").as_str(),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_ident(&mut self, ident: &str) -> CssResult<String> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Ident(s) if s == ident => Ok(s),
            _ => Err(CssError::with_location(
                format!("Expected ident, got {t:?}").as_str(),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_any_ident(&mut self) -> CssResult<String> {
        let t = self.tokenizer.consume();

        match t.token_type {
            TokenType::Delim('.') => {
                let t = self.tokenizer.consume();
                match t.token_type {
                    TokenType::Ident(s) => Ok(format!(".{s}")),
                    _ => Err(CssError::with_location(
                        format!("Expected ident, got {t:?}").as_str(),
                        self.tokenizer.current_location(),
                    )),
                }
            }
            TokenType::Ident(s) => Ok(s),
            _ => Err(CssError::with_location(
                format!("Expected ident, got {t:?}").as_str(),
                self.tokenizer.current_location(),
            )),
        }
    }

    /// Capture the source text from the next token up to (not including) the block's `{`.
    ///
    /// Both ends come from token locations rather than [`Tokenizer::tell`]. The stream
    /// position runs ahead of the parse whenever tokens are sitting in the lookahead queue,
    /// so anchoring on it clipped the first token off the front and pulled the `{` in at the
    /// back - `@supports (display: grid) {` captured as `display: grid) {`. The bug was
    /// invisible while nothing consumed the result.
    pub fn consume_raw_condition(&mut self) -> CssResult<String> {
        let start = self.tokenizer.lookahead(0).location.offset;

        let mut end = None;
        while !self.tokenizer.eof() {
            let t = self.tokenizer.consume();
            if let TokenType::LCurly = t.token_type {
                end = Some(t.location.offset);
                self.tokenizer.reconsume(t);
                break;
            }
        }
        // No block followed, so the condition runs to the end of the input.
        let end = end.unwrap_or_else(|| self.tokenizer.tell());

        Ok(self.tokenizer.slice(start, end.max(start)))
    }
}
