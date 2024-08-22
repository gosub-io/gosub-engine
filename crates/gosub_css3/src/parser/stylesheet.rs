use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    pub fn parse_stylesheet(&mut self) -> Result<Option<Node>, Error> {
        log::trace!("parse_stylesheet");

        let loc = self.tokenizer.current_location();

        let mut children = Vec::new();

        while !self.tokenizer.eof() {
            let t = self.consume_any()?;

            match t.token_type {
                TokenType::Eof => {}
                TokenType::Whitespace(_) => {}
                TokenType::Comment(comment) => {
                    if comment.chars().nth(2) == Some('!') {
                        children.push(Node::new(
                            NodeType::Comment { value: comment },
                            t.location.clone(),
                        ));
                    }
                }
                TokenType::Cdo => {
                    children.push(Node::new(NodeType::Cdo, t.location.clone()));
                }
                TokenType::Cdc => {
                    children.push(Node::new(NodeType::Cdc, t.location.clone()));
                }
                TokenType::AtKeyword(_keyword) => {
                    self.tokenizer.reconsume();

                    let at_rule = self.parse_at_rule(false)?;
                    match at_rule {
                        Some(at_rule_node) => {
                            children.push(at_rule_node);
                        }
                        None => {}  // No valid at-rule found. Ok since we are ignoring errors here
                    }
                }
                _ => {
                    self.tokenizer.reconsume();

                    let rule = self.parse_rule()?;
                    match rule {
                        Some(rule_node) => {
                            children.push(rule_node);
                        }
                        None => {}  // No valid rule found. Ok since we are ignoring errors here
                    }
                }
            }
        }

        for t in self.tokenizer.get_tokens() {
            log::trace!("{:?}", t);
        }
        
        if children.is_empty() {
            return Ok(None);
        }

        Ok(Some(Node::new(NodeType::StyleSheet { children }, loc)))
    }
}
