use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    fn parse_attribute_operator(&mut self) -> Result<Node, Error> {
        log::trace!("parse_attribute_operator");

        let mut value = String::new();
        let loc = self.tokenizer.current_location();

        let c = self.consume_any_delim()?;
        match &c {
            '=' | '~' | '|' | '^' | '$' | '*' => {
                value.push(c);
            }
            _ => {
                self.tokenizer.reconsume();

                return Err(Error::new(
                    format!("Expected attribute operator, got {:?}", c),
                    loc,
                ));
            }
        }

        if c != '=' {
            self.consume_delim('=')?;
            value.push('=');
        }

        Ok(Node::new(NodeType::Operator(value), loc))
    }

    fn parse_class_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_class_selector");

        let loc = self.tokenizer.current_location();

        self.consume(TokenType::Delim('.'))?;

        let value = self.consume_any_ident()?;

        Ok(Node::new(NodeType::ClassSelector { value }, loc))
    }

    fn parse_nesting_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_nesting_selector");

        let loc = self.tokenizer.current_location();

        self.consume(TokenType::Delim('&'))?;

        Ok(Node::new(NodeType::NestingSelector, loc))
    }

    fn parse_type_selector_ident_or_asterisk(&mut self) -> Result<String, Error> {
        let t = self.tokenizer.lookahead(0);
        match t.token_type {
            TokenType::Ident(value) => {
                self.tokenizer.consume();
                Ok(value)
            }
            TokenType::Delim('*') => {
                self.tokenizer.consume();
                Ok("*".to_string())
            }
            _ => Err(Error::new(
                format!("Unexpected token {:?}", t),
                self.tokenizer.current_location(),
            )),
        }
    }

    fn parse_type_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_type_selector");

        let loc = self.tokenizer.current_location();
        let mut value = String::new();

        let t = self.tokenizer.current();
        if t.token_type == TokenType::Delim('|') {
            self.tokenizer.consume();
            value.push('|');
            value.push_str(self.parse_type_selector_ident_or_asterisk()?.as_str());
        } else {
            value.push_str(self.parse_type_selector_ident_or_asterisk()?.as_str());

            let t = self.tokenizer.current();
            if t.token_type == TokenType::Delim('|') {
                self.tokenizer.consume();
                value.push('|');
                value.push_str(self.parse_type_selector_ident_or_asterisk()?.as_str());
            }
        }

        let (namespace, ident) = match value.split_once('|') {
            Some((namespace, ident)) => (Some(namespace.to_string()), ident.to_string()),
            None => (None, value.to_string()),
        };

        Ok(Node::new(
            NodeType::TypeSelector {
                namespace,
                value: ident,
            },
            loc,
        ))
    }

    fn parse_attribute_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_attribute_selector");

        let loc = self.tokenizer.current_location();

        let mut flags = String::new();
        let mut matcher = None;
        let mut value = String::new();

        self.consume(TokenType::LBracket)?;
        self.consume_whitespace_comments();

        let name = self.consume_any_ident()?;
        self.consume_whitespace_comments();

        let t = self.tokenizer.lookahead(0);
        if t.token_type != TokenType::RBracket {
            if !t.is_ident() {
                let op = self.parse_attribute_operator()?;
                matcher = Some(op);

                self.consume_whitespace_comments();

                let t = self.tokenizer.lookahead(0);
                if t.is_string() {
                    value = self.consume_any_string()?;
                } else if t.is_ident() {
                    value = self.consume_any_ident()?;
                } else {
                    return Err(Error::new(
                        format!("Unexpected token {:?}", t),
                        self.tokenizer.current_location(),
                    ));
                }
            }

            self.consume_whitespace_comments();

            let t = self.tokenizer.lookahead(0);
            if t.is_ident() {
                flags = self.consume_any_ident()?;
                self.consume_whitespace_comments();
            }
        }

        self.consume(TokenType::RBracket)?;
        self.consume_whitespace_comments();

        Ok(Node::new(
            NodeType::AttributeSelector {
                name,
                matcher,
                value,
                flags,
            },
            loc,
        ))
    }

    fn parse_id_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_id_selector");

        let loc = self.tokenizer.current_location();

        self.consume(TokenType::Delim('#'))?;

        let t = self.consume_any()?;
        let value = match t.token_type {
            TokenType::Ident(s) => s,
            _ => {
                return Err(Error::new(
                    format!("Unexpected token {:?}", t),
                    self.tokenizer.current_location(),
                ));
            }
        };

        Ok(Node::new(NodeType::IdSelector { value }, loc))
    }

    fn parse_pseudo_element_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_pseudo_element_selector");

        let loc = self.tokenizer.current_location();

        self.consume(TokenType::Colon)?;
        self.consume(TokenType::Colon)?;

        let t = self.tokenizer.lookahead(0);
        let value = if t.is_ident() {
            self.consume_any_ident()?
        } else {
            return Err(Error::new(
                format!("Unexpected token {:?}", t),
                self.tokenizer.current_location(),
            ));
        };

        Ok(Node::new(NodeType::PseudoElementSelector { value }, loc))
    }

    fn parse_pseudo_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_pseudo_selector");

        let loc = self.tokenizer.current_location();

        self.consume(TokenType::Colon)?;

        let t = self.tokenizer.consume();
        let value = match t.token_type {
            TokenType::Ident(value) => Node::new(NodeType::Ident { value }, t.location),
            TokenType::Function(name) => {
                let name = name.to_lowercase();
                let args = self.parse_pseudo_function(name.as_str())?;
                self.consume(TokenType::RParen)?;

                Node::new(
                    NodeType::Function {
                        name,
                        arguments: vec![args],
                    },
                    t.location,
                )
            }
            _ => {
                return Err(Error::new(
                    format!("Unexpected token {:?}", t),
                    self.tokenizer.current_location(),
                ));
            }
        };

        Ok(Node::new(NodeType::PseudoClassSelector { value }, loc))
    }

    pub fn parse_selector(&mut self) -> Result<Node, Error> {
        log::trace!("parse_selector");

        let loc = self.tokenizer.current_location();
        log::trace!("loc: {:?}", loc);

        let mut children = vec![];

        // When true, we have encountered a space which means we need to emit a descendant combinator
        let mut space = false;
        let mut whitespace_location = loc.clone();

        let mut skip_space = false;

        while !self.tokenizer.eof() {
            let t = self.consume_any()?;
            if t.is_comment() {
                continue;
            }

            if skip_space {
                if t.is_whitespace() {
                    continue;
                } else {
                    skip_space = false;
                }
            }

            if t.is_whitespace() {
                // on whitespace for selector
                whitespace_location = t.location.clone();
                space = true;
                continue;
            }

            // let t = self.consume_any()?;
            let child = match t.token_type {
                TokenType::LBracket => {
                    self.tokenizer.reconsume();
                    self.parse_attribute_selector()?
                }
                TokenType::Hash(value) => Node::new(NodeType::IdSelector { value }, t.location),
                TokenType::Colon => {
                    let nt = self.tokenizer.lookahead(0);
                    if nt.token_type == TokenType::Colon {
                        self.tokenizer.reconsume();
                        self.parse_pseudo_element_selector()?
                    } else {
                        self.tokenizer.reconsume();
                        self.parse_pseudo_selector()?
                    }
                }
                TokenType::Ident(value) => Node::new(NodeType::Ident { value }, t.location),

                TokenType::Number(value) => Node::new(NodeType::Number { value }, t.location),

                TokenType::Percentage(value) => {
                    Node::new(NodeType::Percentage { value }, t.location)
                }

                TokenType::Dimension { value, unit } => {
                    Node::new(NodeType::Dimension { value, unit }, t.location)
                }

                TokenType::Delim('+')
                | TokenType::Delim('>')
                | TokenType::Delim('~')
                | TokenType::Delim('/') => {
                    // Dont add descendant combinator since we are now adding another one
                    space = false;

                    self.tokenizer.reconsume();
                    self.parse_combinator()?
                }

                TokenType::Delim('.') => {
                    self.tokenizer.reconsume();
                    self.parse_class_selector()?
                }
                TokenType::Delim('|') | TokenType::Delim('*') => {
                    self.tokenizer.reconsume();
                    self.parse_type_selector()?
                }
                TokenType::Delim('#') => {
                    self.tokenizer.reconsume();
                    self.parse_id_selector()?
                }
                TokenType::Delim('&') => {
                    self.tokenizer.reconsume();
                    self.parse_nesting_selector()?
                }
                TokenType::Comma => {
                    skip_space = true;

                    Node::new(NodeType::Comma, t.location)
                }
                _ => {
                    self.tokenizer.reconsume();
                    break;
                }
            };

            if space {
                // Detected a space previously, so we need to emit a descendant combinator
                let node = Node::new(
                    NodeType::Combinator {
                        value: " ".to_string(),
                    },
                    whitespace_location.clone(),
                );
                // insert before the last added node
                children.push(node);
                space = false;
            }

            children.push(child);
        }

        Ok(Node::new(NodeType::Selector { children }, loc))
    }
}
