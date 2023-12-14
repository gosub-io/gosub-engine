use crate::css3::node::{Node, NodeType};
use crate::css3::tokenizer::TokenType;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    fn parse_class_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_class_selector");

        let loc = self.tokenizer.current_location().clone();

        self.consume(TokenType::Delim('.'))?;

        let value = self.consume_any_ident()?;

        Ok(Node::new(NodeType::ClassSelector { value }, loc))
    }

    fn parse_nesting_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_nesting_selector");

        let loc = self.tokenizer.current_location().clone();

        self.consume(TokenType::Delim('&'))?;

        Ok(Node::new(NodeType::NestingSelector, loc))
    }

    fn parse_type_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_type_selector");
        todo!();
        // let value;

        // entrypoint is either | or *
        // it can be:
        //   *|E
        //   |E

        // let t = self.consume_any();
        //
        // if t.token_type == TokenType::Delim('|') {
        //     let t = self.consume_any();
        //     // eat identifier or asterisk
        // } else {
        //     // eat identifier or asterisk
        //
        //     let t = self.consume_any();
        //     if t.token_type == TokenType::Delim('|') {
        //         let t = self.consume_any();
        //         // eat identifier or asterisk
        //     }
        // }
        //
        // Ok(Node::new(NodeType::TypeSelector))
    }

    fn parse_attribute_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_attribute_selector");

        let loc = self.tokenizer.current_location().clone();

        let mut flags = String::new();
        let mut matcher = None;
        let mut value = String::new();

        self.consume(TokenType::LBracket)?;
        self.consume_whitespace_comments();

        let name = self.consume_any_ident()?;
        self.consume_whitespace_comments();

        let t = self.consume_any()?;
        match t.token_type {
            TokenType::RBracket => {
                self.tokenizer.reconsume();
            }
            TokenType::Ident(value) => {
                flags = value;
            }
            _ => {
                self.tokenizer.reconsume();
                let op = self.parse_operator()?;
                matcher = Some(op);
                self.consume_whitespace_comments();

                let t = self.consume_any()?;
                value = if let TokenType::Ident(s) = t.token_type {
                    s
                } else {
                    return Err(Error::new(
                        format!("Unexpected token {:?}", t),
                        self.tokenizer.current_location().clone(),
                    ));
                };
            }
        }

        self.consume(TokenType::RBracket)?;
        self.consume_whitespace_comments();

        Ok(Node::new(NodeType::AttributeSelector{ name, matcher, value, flags }, loc))
    }

    fn parse_id_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_id_selector");

        let loc = self.tokenizer.current_location().clone();

        self.consume(TokenType::Delim('#'))?;

        let t = self.consume_any()?;
        let value = match t.token_type {
            TokenType::Ident(s) => s,
            _ => {
                return Err(Error::new(
                    format!("Unexpected token {:?}", t),
                    self.tokenizer.current_location().clone(),
                ));
            }
        };

        Ok(Node::new(NodeType::IdSelector { value }, loc))
    }

    fn parse_pseudo_element_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_pseudo_element_selector");

        let loc = self.tokenizer.current_location().clone();

        self.consume(TokenType::Colon)?;
        self.consume(TokenType::Colon)?;

        let t = self.tokenizer.lookahead(0);
        let value = if t.is_ident() {
            self.consume_any_ident()?
        // } else if t.is_function() {
        //
        //     // name is already in T
        //
        //     if let TokenType::Function(name) = t.token_type {
        //         let name = name.to_lowercase().as_str();
        //     } else {
        //         return Err(Error::new(format!("Unexpected token {:?}", t), self.tokenizer.current_location().clone()));
        //     }
        //
        //     self.consume(TokenType::RParen)?;
        } else {
            return Err(Error::new(
                format!("Unexpected token {:?}", t),
                self.tokenizer.current_location().clone(),
            ));
        };

        Ok(Node::new(NodeType::PseudoElementSelector { value }, loc))
    }

    fn parse_pseudo_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_pseudo_selector");

        let loc = self.tokenizer.current_location().clone();

        self.consume(TokenType::Colon)?;

        let t = self.tokenizer.consume();
        let value = match t.token_type {
            TokenType::Ident(value) => {
                Node::new(NodeType::Ident { value }, t.location)
            }
            TokenType::Function(name) => {
                let name = name.to_lowercase();
                let args = self.parse_pseudo_function(name.as_str())?;
                self.consume(TokenType::RParen)?;

                Node::new(NodeType::Function { name, arguments: vec![args] }, t.location)
            }
            _ => {
                return Err(Error::new(
                    format!("Unexpected token {:?}", t),
                    self.tokenizer.current_location().clone(),
                ));
            }
        };

        Ok(Node::new(NodeType::PseudoClassSelector { value }, loc))
    }

    pub fn parse_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_selector");

        let loc = self.tokenizer.current_location().clone();

        let mut children = vec![];

        while !self.tokenizer.eof() {
            self.consume_whitespace_comments();

            let t = self.consume_any()?;
            match t.token_type {
                TokenType::LBracket => {
                    self.tokenizer.reconsume();
                    let selector = self.parse_attribute_selector()?;
                    children.push(selector);
                }
                TokenType::IDHash(value) => {
                    let node = Node::new(NodeType::IdSelector { value }, t.location);
                    children.push(node);
                }
                TokenType::Hash(value) => {
                    let node = Node::new(NodeType::IdSelector { value }, t.location);
                    children.push(node);
                }
                TokenType::Colon => {
                    let nt = self.tokenizer.lookahead(0);
                    if nt.token_type == TokenType::Colon {
                        self.tokenizer.reconsume();
                        let selector = self.parse_pseudo_element_selector()?;
                        children.push(selector);
                    } else {
                        self.tokenizer.reconsume();
                        let selector = self.parse_pseudo_selector()?;
                        children.push(selector);
                    }
                }
                TokenType::Ident(value) => {
                    let node = Node::new(NodeType::Ident { value }, t.location);
                    children.push(node);
                }

                TokenType::Number(value) => {
                    let node = Node::new(NodeType::Number { value }, t.location);
                    children.push(node);
                }

                TokenType::Percentage(value) => {
                    let node = Node::new(NodeType::Percentage { value }, t.location);
                    children.push(node);
                }

                TokenType::Dimension { value, unit } => {
                    let node = Node::new(NodeType::Dimension { value, unit }, t.location);
                    children.push(node);
                }

                TokenType::Delim('+')
                | TokenType::Delim('>')
                | TokenType::Delim('~')
                | TokenType::Delim('/') => {
                    self.tokenizer.reconsume();
                    let node = self.parse_combinator()?;
                    children.push(node);
                }

                TokenType::Delim('.') => {
                    self.tokenizer.reconsume();
                    let selector = self.parse_class_selector()?;
                    children.push(selector);
                }
                TokenType::Delim('|') | TokenType::Delim('*') => {
                    self.tokenizer.reconsume();
                    let selector = self.parse_type_selector()?;
                    children.push(selector);
                }
                TokenType::Delim('#') => {
                    self.tokenizer.reconsume();
                    let selector = self.parse_id_selector()?;
                    children.push(selector);
                }
                TokenType::Delim('&') => {
                    self.tokenizer.reconsume();
                    let selector = self.parse_nesting_selector()?;
                    children.push(selector);
                }

                _ => {
                    self.tokenizer.reconsume();
                    break;
                }
            }
        }

        Ok(Node::new(NodeType::Selector{ children }, loc))
    }
}
