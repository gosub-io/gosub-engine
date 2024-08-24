use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    pub fn parse_property_name(&mut self) -> Result<String, Error> {
        log::trace!("parse_property_name");
        let t = self.consume_any()?;
        match t.token_type {
            TokenType::Delim('*')
            | TokenType::Delim('$')
            | TokenType::Delim('+')
            | TokenType::Delim('#')
            | TokenType::Delim('&') => {} //next
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
            _ => Err(Error::new(
                format!("Unexpected token {:?}", t),
                self.tokenizer.current_location(),
            )),
        }
    }

    pub fn parse_declaration(&mut self) -> Result<Option<Node>, Error> {
        log::trace!("parse_declaration");

        let result = self.parse_declaration_internal();
        if result.is_err() && self.config.ignore_errors {
            log::warn!("Ignoring error in parse_declaration: {:?}", result);
            self.parse_until_declaration_end();
            return Ok(None);
        }

        if let Ok(declaration) = result {
            return Ok(Some(declaration));
        }
        Ok(None)
    }

    fn parse_declaration_internal(&mut self) -> Result<Node, Error> {
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

        if value.is_empty() {
            return Err(Error::new(
                "Expected value in declaration".to_string(),
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

    fn parse_until_declaration_end(&mut self) {
        log::trace!(
            "parse_until_declaration_end, now at: {:?}",
            self.tokenizer.current_location()
        );
        loop {
            let t = self.consume_any();
            if t.is_err() {
                break;
            }
            match t.unwrap().token_type {
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
