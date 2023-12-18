use crate::css3::node::{Node, NodeType};
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    fn parse_declaration_custom_property(&mut self) -> Result<Node, Error> {
        log::trace!("parse_declaration_custom_property");
        let loc = self.tokenizer.current_location().clone();

        let n = Node::new(NodeType::String {
            value: "custom_property".to_string(),
        }, loc.clone());

        Ok(Node::new(NodeType::Value { children: vec![n] }, loc))
    }

    pub fn parse_property_name(&mut self) -> Result<String, Error> {
        log::trace!("parse_property_name");
        let t = self.consume_any()?;
        match t.token_type {
            TokenType::Delim('*') => { // next
            }
            TokenType::Delim('$') => { // next
            }
            TokenType::Delim('+') => { // next
            }
            TokenType::Delim('#') => { // next
            }
            TokenType::Delim('&') => { // next
            }
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
                self.tokenizer.current_location().clone(),
            )),
        }
    }

    pub fn parse_declaration(&mut self) -> Result<Node, Error> {
        log::trace!("parse_declaration");

        let loc = self.tokenizer.current_location().clone();

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

        let t = self.consume_any()?;
        if t.is_delim('!') {
            self.consume_ident("important")?;
            self.consume_whitespace_comments();

            important = true;
        } else {
            self.tokenizer.reconsume();
        }

        Ok(Node::new(NodeType::Declaration{ property, value, important }, loc))
    }
}

fn matching_end_token(end_token_type: TokenType, start_token_type: TokenType) -> bool {
    match start_token_type {
        TokenType::LCurly => end_token_type == TokenType::RCurly,
        TokenType::LParen => end_token_type == TokenType::RParen,
        TokenType::LBracket => end_token_type == TokenType::RBracket,
        _ => false,
    }
}
