use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::Css3;
use gosub_shared::errors::CssResult;

impl Css3<'_> {
    pub fn parse_calc(&mut self) -> CssResult<Node> {
        log::trace!("parse_calc");

        let loc = self.tokenizer.current_location();

        let expr = self.parse_calc_expr()?;

        Ok(Node::new(NodeType::Calc { expr: Box::new(expr) }, loc))
    }

    fn parse_calc_expr(&mut self) -> CssResult<Node> {
        log::trace!("parse_calc_expr");

        let loc = self.tokenizer.current_location();

        // Rebuild the expression text from the consumed TOKENS. Slicing the raw stream
        // does not work here: the tokenizer pre-tokenizes into a buffer, so the stream
        // position runs ahead of the current token and the slice comes up empty.
        let mut expr = String::new();

        loop {
            let t = self.consume_any()?;
            match t.token_type {
                TokenType::Eof => break,
                // A nested function or `(` opens a sub-expression, one recursion level
                // deeper. The recursive call consumes through the matching `)`.
                TokenType::Function(name) => {
                    expr.push_str(&name);
                    expr.push('(');
                    let inner = self.recurse(Self::parse_calc_expr)?;
                    if let NodeType::Raw { value } = inner.node_type {
                        expr.push_str(&value);
                    }
                    expr.push(')');
                }
                TokenType::LParen => {
                    expr.push('(');
                    let inner = self.recurse(Self::parse_calc_expr)?;
                    if let NodeType::Raw { value } = inner.node_type {
                        expr.push_str(&value);
                    }
                    expr.push(')');
                }
                TokenType::RParen => break,
                TokenType::Comment(_) => {}
                _ => {
                    // Token's Display form is its CSS serialization (whitespace -> " ").
                    expr.push_str(&t.to_string());
                }
            }
        }

        Ok(Node::new(
            NodeType::Raw {
                value: expr.trim().to_string(),
            },
            loc,
        ))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_parse_calc() {
        // test!(parse_calc, "calc(1px + 2px)", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        // test!(parse_calc, "calc(100px + (200px - 100px) * ((100vh - 500px) / (800 - 500)))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        // test!(parse_calc, "calc(12px + (20 - 12) * ((100vw - 300px) / (700 - 300))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        // test!(parse_calc, "calc(50px + 5 * (100vw - 300px) / (1100 - 300)), 1fr)", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        // test!(parse_calc, "calc(10px + 20 * (100vw - 300px) / (1100 - 300))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        // test!(parse_calc, "calc(20px + (50px - 20px) * ((100vw - 600px) / (1000 - 600)))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        // test!(parse_calc, "calc(100px + (200px - 100px) * ((100vh - 500px) / (800 - 500))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        // test!(parse_calc, "calc(40% + calc(82vw / 53em))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        // test!(parse_calc, "calc(52% / 48px)", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        // test!(parse_calc, "calc((100% - var(--maxContainerWidth)) / 2)", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
    }
}
