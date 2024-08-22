use crate::node::{Node, NodeType};
use crate::tokenizer::TokenType;
use crate::{Css3, Error};

impl Css3<'_> {
    pub fn parse_calc(&mut self) -> Result<Node, Error> {
        log::trace!("parse_calc");

        let loc = self.tokenizer.current_location();

        let expr = self.parse_calc_expr()?;

        Ok(Node::new(NodeType::Calc { expr }, loc))
    }

    fn parse_calc_expr(&mut self) -> Result<Node, Error> {
        log::trace!("parse_calc_expr");

        let loc = self.tokenizer.current_location();

        let start = self.tokenizer.tell();

        loop {
            let t = self.consume_any()?;
            match t.token_type {
                TokenType::Eof => break,
                TokenType::Function(_) => {
                    self.parse_calc_expr()?;
                }
                TokenType::LParen => {
                    self.parse_calc_expr()?;
                }
                TokenType::RParen => break,
                _ => {
                    // ignore
                }
            }
        }

        let end = self.tokenizer.tell();

        let expr = self.tokenizer.slice(start, end);

        Ok(Node::new(NodeType::Raw { value: expr }, loc))
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
