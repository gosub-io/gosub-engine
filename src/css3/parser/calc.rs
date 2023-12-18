use crate::css3::node::{Node, NodeType};
use crate::css3::{Css3, Error};
use crate::css3::tokenizer::TokenType;

pub enum Nearest {
    None,
    Nearest,
    Up,
    Down,
    ToZero,
}

impl Css3<'_> {
    pub fn parse_calc(&mut self) -> Result<Node, Error> {
        log::trace!("parse_calc");

        let loc = self.tokenizer.current_location().clone();

        let expr = self.parse_calc_expr()?;

        Ok(Node::new(NodeType::Calc{expr}, loc))
    }

    fn parse_calc_sum(&mut self) -> Result<Node, Error> {
        let t = self.consume_any()?;

        let loc = self.tokenizer.current_location().clone();

        match t.token_type {
            // This doesn't work correctly..  now:   calc( * 12px + 12) is parsed
            TokenType::Delim('+') => {
                let expr1 = self.parse_calc_product()?;
                let expr2 = self.parse_calc_sum()?;
                Ok(Node::new(NodeType::CalcSum{ expr: Box::new(expr1), expr2: Box::new(expr2) }, loc))
            }
            TokenType::Delim('-') => {
                let expr1 = self.parse_calc_product()?;
                let expr2 = self.parse_calc_sum()?;
                Ok(Node::new(NodeType::CalcSum{ expr: Box::new(expr1), expr2: Box::new(expr2) }, loc))
            }
            TokenType::Delim('*') => {
                let expr1 = self.parse_calc_product()?;
                let expr2 = self.parse_calc_sum()?;
                Ok(Node::new(NodeType::CalcSum{ expr: Box::new(expr1), expr2: Box::new(expr2) }, loc))
            }
            TokenType::Delim('/') => {
                let expr1 = self.parse_calc_product()?;
                let expr2 = self.parse_calc_sum()?;
                Ok(Node::new(NodeType::CalcSum{ expr: Box::new(expr1), expr2: Box::new(expr2) }, loc))
            }

            TokenType::LParen => {
                let expr = self.parse_calc_sum()?;
                self.consume(TokenType::RParen)?;
                Ok(Node::new(NodeType::CalcSum{ expr}, loc))
            }
            TokenType::Percentage(value) => {
                Ok(Node::new(NodeType::Percentage{value}, t.location))
            }
            TokenType::Number(value) => {
                Ok(Node::new(NodeType::Number{value}, t.location))
            }
            TokenType::Dimension { value, unit } => {
                Ok(Node::new(NodeType::Dimension { value, unit }, t.location))
            }
            TokenType::Ident(value) if value.to_ascii_lowercase() == "e" => {
                Ok(Node::new(NodeType::Ident { value: "e".to_string() }, t.location))
            }
            TokenType::Ident(value) if value.to_ascii_lowercase() == "pi" => {
                Ok(Node::new(NodeType::Ident { value: "pi".to_string() }, t.location))
            }
            TokenType::Ident(value) if value.to_ascii_lowercase() == "infinity" => {
                Ok(Node::new(NodeType::Ident { value: "infinity".to_string() }, t.location))
            }
            TokenType::Ident(value) if value.to_ascii_lowercase() == "-infinity" => {
                Ok(Node::new(NodeType::Ident { value: "-infinity".to_string() }, t.location))
            }
            TokenType::Ident(value) if value.to_ascii_lowercase() == "nan" => {
                Ok(Node::new(NodeType::Ident { value: "nan".to_string() }, t.location))
            }
            _ => {
            }
        }

        Err(Error::new(
            format!("Unexpected token {:?}", t),
            self.tokenizer.current_location().clone(),
        ))
    }

    fn parse_calc_expr(&mut self) -> Result<Node, Error> {
        let loc = self.tokenizer.current_location().clone();

        let t = self.consume_any()?;
        let node = match t.token_type {
            TokenType::Function(name) if name == "calc" => {
                Node::new(NodeType::CalcCalc{
                    expr: self.parse_calc_sum()?
                })
            }
            TokenType::Function(name) if name == "min" => {

                Node::new(NodeType::CalcMin{
                    expr: self.parse_calc_sum_list(1, None)
                })
            }
            TokenType::Function(name) if name == "max" => {
                self.parse_calc_sum_list(1, None);
                Node::new(NodeType::CalcMax{
                    expr: self.parse_calc_sum_list(1, None)
                })
            }
            TokenType::Function(name) if name == "clamp" => {
                Node::new(NodeType::CalcClamp{
                    expr1: self.parse_calc_sum()?,
                    expr2: self.parse_calc_sum()?,
                    expr3: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "round" => {
                let mut nearest = Nearest::None;

                let t = self.tokenizer.lookahead(1);
                if let TokenType::Ident(value) = t.token_type {
                    nearest = match value.as_str() {
                        "nearest" => {
                            self.consume_any_ident();
                            Nearest::Nearest
                        },
                        "up" => {
                            self.consume_any_ident();
                            Nearest::Up
                        },
                        "down" => {
                            self.consume_any_ident();
                            Nearest::Down
                        },
                        "to-zero" => {
                            self.consume_any_ident();
                            Nearest::ToZero
                        },
                        _ => Nearest::None,
                    };
                }
                let a = self.parse_calc_sum()?;
                let b = self.parse_calc_sum()?;

                Node::new(NodeType::CalcRound{
                    rounding: nearest,
                    expr1: self.parse_calc_sum()?,
                    expr2: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "mod" => {
                Node::new(NodeType::CalcMod{
                    expr1: self.parse_calc_sum()?,
                    expr2: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "rem" => {
                Node::new(NodeType::CalcRem{
                    expr1: self.parse_calc_sum()?,
                    expr2: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "sin" => {
                Node::new(NodeType::CalcSin{
                    expr: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "cos" => {
                Node::new(NodeType::CalcCos{
                    expr: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "tan" => {
                Node::new(NodeType::CalcTan{
                    expr: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "asin" => {
                Node::new(NodeType::CalcASin{
                    expr: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "acos" => {
                Node::new(NodeType::CalcACos{
                    expr: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "atan" => {
                Node::new(NodeType::CalcATan{
                    expr: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "atan2" => {
                Node::new(NodeType::CalcATan2 {
                    expr1: self.parse_calc_sum()?,
                    expr2: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "pow" => {
                Node::new(NodeType::CalcPow {
                    expr1: self.parse_calc_sum()?,
                    expr2: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "sqrt" => {
                Node::new(NodeType::CalcSqrt {
                    expr: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "hypot" => {
                Node::new(NodeType::CalcHypot{
                    expr: self.parse_calc_sum_list()?,
                })
            }
            TokenType::Function(name) if name == "log" => {
                Node::new(NodeType::CalcLog {
                    expr1: self.parse_calc_sum()?,
                    expr2: self.parse_calc_sum_optional(),
                })
            }
            TokenType::Function(name) if name == "exp" => {
                Node::new(NodeType::CalcExp {
                    expr1: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "abs" => {
                Node::new(NodeType::CalcAbs {
                    expr1: self.parse_calc_sum()?,
                })
            }
            TokenType::Function(name) if name == "sign" => {
                Node::new(NodeType::CalcSign {
                    expr1: self.parse_calc_sum()?,
                })
            }
        };

        Ok(Node::new(NodeType::Calc{ expr: node }, loc))
    }
}

#[cfg(test)]
mod tests {
    use simple_logger::SimpleLogger;
    use crate::byte_stream::Stream;
    use super::*;

    macro_rules! test {
        ($func:ident, $input:expr, $expected:expr) => {
            let mut it = crate::css3::ByteStream::new();
            it.read_from_str($input, Some(crate::byte_stream::Encoding::UTF8));
            it.close();

            // let mut tokenizer = crate::css3::tokenizer::Tokenizer::new(&mut it, crate::css3::Location::default());
            let mut parser = crate::css3::Css3::new(&mut it);
            let result = parser.$func().unwrap();

            assert_eq!(result.node_type, $expected);
        };
    }

    #[test]
    fn test_parse_calc() {
        SimpleLogger::new().init().unwrap();

        test!(parse_calc, "calc(1px + 2px)", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        test!(parse_calc, "calc(100px + (200px - 100px) * ((100vh - 500px) / (800 - 500)))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        test!(parse_calc, "calc(12px + (20 - 12) * ((100vw - 300px) / (700 - 300))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        test!(parse_calc, "calc(50px + 5 * (100vw - 300px) / (1100 - 300)), 1fr)", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        test!(parse_calc, "calc(10px + 20 * (100vw - 300px) / (1100 - 300))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        test!(parse_calc, "calc(20px + (50px - 20px) * ((100vw - 600px) / (1000 - 600)))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        test!(parse_calc, "calc(100px + (200px - 100px) * ((100vh - 500px) / (800 - 500))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        test!(parse_calc, "calc(40% + calc(82vw / 53em))", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        test!(parse_calc, "calc(52% / 48px)", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
        test!(parse_calc, "calc((100% - var(--maxContainerWidth)) / 2)", Box::new(NodeType::AnPlusB { a: "1".to_string(), b: "2".to_string() } ));
    }
}
