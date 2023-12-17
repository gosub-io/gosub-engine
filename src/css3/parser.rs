use crate::css3::tokenizer::{Number, Token, TokenType};
use crate::css3::{Css3, Error};

mod at_rule;
mod block;
mod combinator;
mod declaration;
mod function;
mod operator;
mod rule;
mod selector;
mod selector_list;
mod stylesheet;
mod url;
mod value;
mod pseudo;
mod anplusb;

impl Css3<'_> {
    /// Consumes a specific token
    pub fn consume(&mut self, token_type: TokenType) -> Result<Token, Error> {
        let t = self.tokenizer.consume();
        if t.token_type != token_type {
            return Err(Error::new(
                format!("Expected {:?}, got {:?}", token_type, t),
                self.tokenizer.current_location().clone(),
            ));
        }

        Ok(t)
    }

    /// Consumes any token
    pub fn consume_any(&mut self) -> Result<Token, Error> {
        Ok(self.tokenizer.consume())
    }

    pub fn consume_function(&mut self) -> Result<String, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Function(name) => Ok(name),
            _ => Err(Error::new(
                format!("Expected function, got {:?}", t),
                self.tokenizer.current_location().clone(),
            )),
        }
    }

    pub fn consume_any_number(&mut self) -> Result<Number, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Number(value) => Ok(value),
            _ => Err(Error::new(
                format!("Expected number, got {:?}", t),
                self.tokenizer.current_location().clone(),
            )),
        }
    }

    pub fn consume_any_delim(&mut self) -> Result<char, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Delim(c) => Ok(c),
            _ => Err(Error::new(
                format!("Expected delimiter, got {:?}", t),
                self.tokenizer.current_location().clone(),
            )),
        }
    }

    pub fn consume_delim(&mut self, delimiter: char) -> Result<char, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Delim(c) if c == delimiter => Ok(c),
            _ => Err(Error::new(
                format!("Expected delimiter '{}', got {:?}", delimiter, t),
                self.tokenizer.current_location().clone(),
            )),
        }
    }

    pub fn consume_whitespace_comments(&mut self) {
        loop {
            let t = self.tokenizer.consume();
            match t.token_type {
                TokenType::Whitespace | TokenType::Comment(_) => {
                    // just eat it
                }
                _ => {
                    self.tokenizer.reconsume();
                    break;
                }
            }
        }
    }

    pub fn consume_ident(&mut self, ident: &str) -> Result<String, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Ident(s) if s == ident => Ok(s),
            _ => Err(Error::new(
                format!("Expected ident, got {:?}", t),
                self.tokenizer.current_location().clone(),
            )),
        }
    }

    pub fn consume_any_ident(&mut self) -> Result<String, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Ident(s) => Ok(s),
            _ => Err(Error::new(
                format!("Expected ident, got {:?}", t),
                self.tokenizer.current_location().clone(),
            )),
        }
    }
}
