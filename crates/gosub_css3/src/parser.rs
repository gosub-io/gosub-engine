use crate::tokenizer::{Number, Token, TokenType};
use crate::{Css3, Error};

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
    pub fn consume(&mut self, token_type: TokenType) -> Result<Token, Error> {
        let t = self.tokenizer.consume();
        if t.token_type != token_type {
            return Err(Error::new(
                format!("Expected {:?}, got {:?}", token_type, t),
                self.tokenizer.current_location(),
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
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_any_number(&mut self) -> Result<Number, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Number(value) => Ok(value),
            _ => Err(Error::new(
                format!("Expected number, got {:?}", t),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_any_delim(&mut self) -> Result<char, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Delim(c) => Ok(c),
            _ => Err(Error::new(
                format!("Expected delimiter, got {:?}", t),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_any_string(&mut self) -> Result<String, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::QuotedString(s) => Ok(s),
            _ => Err(Error::new(
                format!("Expected string, got {:?}", t),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_delim(&mut self, delimiter: char) -> Result<char, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Delim(c) if c == delimiter => Ok(c),
            _ => Err(Error::new(
                format!("Expected delimiter '{}', got {:?}", delimiter, t),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_whitespace_comments(&mut self) {
        loop {
            let t = self.tokenizer.consume();
            match t.token_type {
                TokenType::Whitespace(_) | TokenType::Comment(_) => {
                    // just eat it
                }
                _ => {
                    self.tokenizer.reconsume();
                    break;
                }
            }
        }
    }

    pub fn consume_ident_ci(&mut self, ident: &str) -> Result<String, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Ident(s) if s.eq_ignore_ascii_case(ident) => Ok(s),
            _ => Err(Error::new(
                format!("Expected ident, got {:?}", t),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_ident(&mut self, ident: &str) -> Result<String, Error> {
        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Ident(s) if s == ident => Ok(s),
            _ => Err(Error::new(
                format!("Expected ident, got {:?}", t),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_any_ident(&mut self) -> Result<String, Error> {
        let t = self.tokenizer.consume();

        match t.token_type {
            TokenType::Delim('.') => {
                let t = self.tokenizer.consume();
                match t.token_type {
                    TokenType::Ident(s) => Ok(format!(".{}", s)),
                    _ => Err(Error::new(
                        format!("Expected ident, got {:?}", t),
                        self.tokenizer.current_location(),
                    )),
                }
            }
            TokenType::Ident(s) => Ok(s),
            _ => Err(Error::new(
                format!("Expected ident, got {:?}", t),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn consume_raw_condition(&mut self) -> Result<String, Error> {
        let start = self.tokenizer.tell();

        while !self.tokenizer.eof() {
            let t = self.tokenizer.consume();
            if let TokenType::LCurly = t.token_type {
                self.tokenizer.reconsume();
                break;
            }
        }
        let end = self.tokenizer.tell();

        Ok(self.tokenizer.slice(start, end))
    }
}
