use crate::css3::node::{Node, NodeType};
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    pub fn parse_value_sequence(&mut self) -> Result<Vec<Node>, Error> {
        log::trace!("parse_value_sequence");

        let mut children = Vec::new();

        while !self.tokenizer.eof() {
            let t = self.consume_any()?;
            match t.token_type {
                TokenType::Comment(_) => {
                    // eat token
                }
                TokenType::Whitespace => {
                    // eat token
                }
                _ => {
                    self.tokenizer.reconsume();
                }
            }

            let child = self.parse_value()?;
            if child.is_none() {
                break;
            }
            let child = child.unwrap();

            children.push(child);
        }

        Ok(children)
    }

    // ok:
    //    some: some value is found
    //    none: no value is found (but this is not an error)
    // err:
    //    parsing went wrong
    fn parse_value(&mut self) -> Result<Option<Node>, Error> {
        log::trace!("parse_value");
        let t = self.consume_any()?;
        match t.token_type {
            TokenType::IDHash(value) => {
                let node = Node::new(NodeType::Ident { value: format!("#{}", value) }, t.location);
                Ok(Some(node))
            }
            TokenType::Hash(value) => {
                let node = Node::new(NodeType::Hash { value }, t.location);
                Ok(Some(node))
            }
            TokenType::Comma => {
                let node = Node::new(NodeType::Operator(",".into()), t.location);
                Ok(Some(node))
            }
            TokenType::LBracket => {
                todo!();
            }
            TokenType::QuotedString(value) => {
                let node = Node::new(NodeType::String { value }, t.location);
                Ok(Some(node))
            }
            TokenType::Dimension { value, unit } => {
                let node = Node::new(NodeType::Dimension { value, unit }, t.location);
                Ok(Some(node))
            }
            TokenType::Percentage(value) => {
                let node = Node::new(NodeType::Percentage { value }, t.location);
                Ok(Some(node))
            }
            TokenType::Number(value) => {
                let node = Node::new(NodeType::Number { value }, t.location);
                Ok(Some(node))
            }
            TokenType::Function(name) => {
                let node = if name.eq_ignore_ascii_case("url") {
                    // it would make sense if this would start at url("") instead of the actual string
                    self.tokenizer.reconsume();
                    self.parse_url()?
                } else {
                    self.tokenizer.reconsume();
                    self.parse_function()?
                };
                Ok(Some(node))
            }
            TokenType::Url(url) => {
                let node = Node::new(NodeType::Url { url }, t.location);
                Ok(Some(node))
            }
            TokenType::Ident(value) => {
                if value == "opacity" && self.in_alpha_function {
                    self.consume_delim('=')?;
                    let value = self.consume_any_number()?;
                    let node = Node::new(NodeType::OpacityIE8Hack { value }, t.location);
                    return Ok(Some(node))
                }

                if value.eq_ignore_ascii_case("u+") {
                    // unicode
                    todo!("unicode");
                } else {
                    let node = Node::new(NodeType::Ident { value }, t.location);
                    Ok(Some(node))
                }
            }
            TokenType::Delim(c) => {
                match c {
                    '+' | '-' | '*' | '/' => {
                        self.tokenizer.reconsume();
                        let node = self.parse_operator()?;
                        return Ok(Some(node));
                    }
                    '#' => {
                        Err(Error::new(
                            format!("Unexpected token {:?}", t),
                            self.tokenizer.current_location().clone(),
                        ))
                    }
                    _ => {
                        self.tokenizer.reconsume();
                        return Ok(None)
                    }
                }
            }

            _ => {
                self.tokenizer.reconsume();
                Ok(None)
            }
        }
    }
}
