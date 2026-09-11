use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssError;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    pub fn parse_value_sequence(&mut self) -> CssResult<Vec<Node>> {
        log::trace!("parse_value_sequence");

        let mut children: Vec<Node> = Vec::new();

        while !self.tokenizer.eof() {
            // Skip the run of whitespace and comments ahead of the next value, remembering
            // whether any whitespace was among it. Whitespace is not a value, but css-values-4
            // §10.1 makes its presence part of the meaning of `+` and `-`: both require
            // whitespace on either side, and that is the only thing separating
            // `calc(1px - 2px)` (a subtraction) from `calc(1px -2px)` (two adjacent values).
            // Discarding it outright left that rule unenforceable for every math function whose
            // arguments come through here.
            //
            // This loop also replaces a single-token skip that could not cope with whitespace
            // and a comment in sequence: `1px /* c */ 2px` ended the value list at the comment.
            let mut space_before = false;
            loop {
                let t = self.consume_any()?;
                match t.token_type {
                    TokenType::Whitespace(_) => space_before = true,
                    TokenType::Comment(_) => {}
                    _ => {
                        self.tokenizer.reconsume(t);
                        break;
                    }
                }
            }

            // Whitespace ahead of this value is also whitespace *after* whatever preceded it.
            if space_before {
                if let Some(NodeType::Operator { space_after, .. }) =
                    children.last_mut().map(|node| &mut node.node_type)
                {
                    *space_after = true;
                }
            }

            let Some(mut child) = self.parse_value()? else {
                break;
            };
            if let NodeType::Operator {
                space_before: before, ..
            } = &mut child.node_type
            {
                *before = space_before;
            }
            children.push(child);
        }

        Ok(children)
    }

    // ok:
    //    some: some value is found
    //    none: no value is found (but this is not an error)
    // err:
    //    parsing went wrong
    fn parse_value(&mut self) -> CssResult<Option<Node>> {
        log::trace!("parse_value");

        let t = self.consume_any()?;
        match t.token_type {
            TokenType::Hash(value) => {
                let node = Node::new(NodeType::Hash { value }, t.location);
                Ok(Some(node))
            }
            TokenType::Comma => {
                let node = Node::new(NodeType::Comma, t.location);
                Ok(Some(node))
            }
            TokenType::LBracket => Err(CssError::with_location(
                "Unexpected token [",
                self.tokenizer.current_location(),
            )),
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
            TokenType::Function(ref name) => {
                let node = if name.eq_ignore_ascii_case("calc") {
                    self.parse_calc()?
                } else if name.eq_ignore_ascii_case("url") {
                    self.tokenizer.reconsume(t);
                    self.parse_url()?
                } else {
                    self.tokenizer.reconsume(t);
                    self.parse_function()?
                };
                Ok(Some(node))
            }
            TokenType::Url(url) => {
                let node = Node::new(NodeType::Url { url }, t.location);
                Ok(Some(node))
            }
            TokenType::UnicodeRange(value) => {
                // Stored as a string node (e.g. for the `unicode-range` @font-face descriptor).
                let node = Node::new(NodeType::String { value }, t.location);
                Ok(Some(node))
            }
            TokenType::Ident(value) => {
                if value.eq_ignore_ascii_case("progid") {
                    let _ = self.consume(TokenType::Colon)?;
                    let _ = self.consume_ident_ci("dximagetransform")?;
                    let _ = self.consume_delim('.')?;
                    let _ = self.consume_ident_ci("microsoft")?;
                    let _ = self.consume_delim('.')?;
                    self.allow_values_in_argument_list.push(true);
                    let func = self.parse_function()?;
                    self.allow_values_in_argument_list.pop();
                    let n = Node::new(NodeType::MSFunction { func: Box::new(func) }, t.location);

                    return Ok(Some(n));
                }

                if !self.allow_values_in_argument_list.is_empty() && self.tokenizer.lookahead(0).is_delim('=') {
                    self.consume_delim('=')?;
                    let t = self.consume_any()?;
                    let node = match t.token_type {
                        TokenType::QuotedString(default_value) => Node::new(
                            NodeType::MSIdent {
                                value: value.to_string(),
                                default_value,
                            },
                            t.location,
                        ),
                        TokenType::Number(default_value) => Node::new(
                            NodeType::MSIdent {
                                value: value.to_string(),
                                default_value: default_value.to_string(),
                            },
                            t.location,
                        ),
                        TokenType::Ident(default_value) => {
                            Node::new(NodeType::MSIdent { value, default_value }, t.location)
                        }
                        _ => {
                            return Err(CssError::with_location(
                                format!("Expected number or ident, got {t:?}").as_str(),
                                self.tokenizer.current_location(),
                            ))
                        }
                    };

                    return Ok(Some(node));
                }

                if value.eq_ignore_ascii_case("u+") {
                    // unicode range not yet implemented
                    Err(CssError::with_location(
                        "unicode range values not yet implemented",
                        self.tokenizer.current_location(),
                    ))
                } else {
                    let node = Node::new(NodeType::Ident { value }, t.location);
                    Ok(Some(node))
                }
            }
            TokenType::Delim(c) => match c {
                '+' | '-' | '*' | '/' => {
                    self.tokenizer.reconsume(t);
                    let node = self.parse_operator()?;
                    Ok(Some(node))
                }
                '#' => Err(CssError::with_location(
                    format!("Unexpected token {t:?}").as_str(),
                    self.tokenizer.current_location(),
                )),
                _ => {
                    self.tokenizer.reconsume(t);
                    Ok(None)
                }
            },
            _ => {
                self.tokenizer.reconsume(t);
                Ok(None)
            }
        }
    }
}
