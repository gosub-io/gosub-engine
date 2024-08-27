use crate::node::{Node, NodeType};
use crate::tokenizer::{Number, TokenType};
use crate::{Css3, Error};

impl Css3<'_> {
    fn do_dimension_block(
        &mut self,
        value: Number,
        unit: String,
    ) -> Result<(String, String), Error> {
        log::trace!("do_dimension_block");

        let value = value.to_string();

        if unit.chars().nth(0).unwrap().to_lowercase().to_string() != "n" {
            return Err(Error::new(
                format!("Expected n, found {}", unit).to_string(),
                self.tokenizer.current_location(),
            ));
        }
        Ok(if unit.len() == 1 {
            (value.to_string(), self.parse_anplusb_b()?)
        } else {
            (value.to_string(), unit[1..].to_string())
        })
    }

    fn check_integer(
        &mut self,
        value: &str,
        offset: usize,
        allow_sign: bool,
    ) -> Result<bool, Error> {
        let sign = value
            .chars()
            .nth(offset)
            .unwrap_or(' ')
            .to_lowercase()
            .to_string();
        let mut pos = offset;

        if sign == "+" || sign == "-" {
            if !allow_sign {
                return Err(Error::new(
                    format!("Unexpected sign {}", sign).to_string(),
                    self.tokenizer.current_location(),
                ));
            }
            pos += 1;
        }

        for c in value.chars().skip(pos) {
            if !c.is_ascii_digit() {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn expect_char(&mut self, value: &str, c: &str, offset: usize) -> Result<bool, Error> {
        let nval = value
            .chars()
            .nth(offset)
            .unwrap_or(' ')
            .to_lowercase()
            .to_string();
        if nval != c {
            return Err(Error::new(
                format!("Expected {}", c).to_string(),
                self.tokenizer.current_location(),
            ));
        }

        Ok(true)
    }

    fn parse_anplusb_b(&mut self) -> Result<String, Error> {
        log::trace!("parse_anplusb_b");

        self.consume_whitespace_comments();

        if let TokenType::Eof = self.tokenizer.lookahead(0).token_type {
            return Ok("0".to_string());
        }
        if let TokenType::Semicolon = self.tokenizer.lookahead(0).token_type {
            return Ok("0".to_string());
        }
        if let TokenType::RCurly = self.tokenizer.lookahead(0).token_type {
            return Ok("0".to_string());
        }
        if let TokenType::RParen = self.tokenizer.lookahead(0).token_type {
            return Ok("0".to_string());
        }

        let negative = match self.tokenizer.lookahead(0).token_type {
            TokenType::Delim('-') => {
                self.consume_delim('-')?;
                true
            }
            TokenType::Delim('+') => {
                self.consume_delim('+')?;
                false
            }
            TokenType::Number(_) => {
                // self.tokenizer.reconsume();
                false
            }
            _ => {
                return Err(Error::new(
                    format!(
                        "Expected +, - or number, found {:?}",
                        self.tokenizer.lookahead(0).token_type
                    )
                    .to_string(),
                    self.tokenizer.current_location(),
                ));
            }
        };

        self.consume_whitespace_comments();

        let val = self.consume_any_number()?;
        if negative {
            return Ok(format!("-{}", val));
        }

        Ok(val.to_string())
    }

    fn do_negative_block(&mut self, value: &str) -> Result<(String, String), Error> {
        log::trace!("do_negative_block");

        let a = String::from("-1");
        let mut b = String::new();

        self.expect_char(value, "n", 1)?;

        match value.len() {
            2 => {
                self.consume_any()?;
                b = self.parse_anplusb_b()?;
            }
            3 => {
                self.expect_char(value, "-", 2)?;
                self.consume_any()?;
                self.consume_whitespace_comments();

                self.check_integer(value, 0, false)?;

                b.push('-');
                let s = self.consume_any_number()?.to_string();
                b.push_str(s.as_str());
            }
            _ => {
                self.expect_char(value, "-", 2)?;
                self.check_integer(value, 3, false)?;
                b.push_str("foobar");
            }
        }

        Ok((a, b))
    }

    fn do_plus_block(&mut self, value: &str) -> Result<(String, String), Error> {
        log::trace!("do_plus_block");

        let a = String::from("1");
        let mut b = String::new();

        self.expect_char(value, "n", 0)?;

        match value.len() {
            1 => {
                self.consume_any()?;
                b = self.parse_anplusb_b()?;
            }
            2 => {
                self.expect_char(value, "-", 1)?;
                self.consume_any()?;
                self.consume_whitespace_comments();

                self.check_integer(value, 0, false)?;

                b.push('-');
                let s = self.consume_any_number()?.to_string();
                b.push_str(s.as_str());
            }
            _ => {
                self.expect_char(value, "-", 1)?;
                self.check_integer(value, 2, false)?;
                b.push_str("foobar");
            }
        }

        Ok((a, b))
    }

    pub fn parse_anplusb(&mut self) -> Result<Node, Error> {
        log::trace!("parse_anplusb");

        let loc = self.tokenizer.current_location();

        let mut a = String::from("1");
        let mut b;

        let t = self.tokenizer.consume();
        match t.token_type {
            TokenType::Number(_) => {
                self.tokenizer.reconsume();
                // self.check_integer(value, 0, true);
                b = self.consume_any_number()?.to_string();
            }
            TokenType::Ident(value) if value.starts_with('-') => {
                self.tokenizer.reconsume();
                (a, b) = self.do_negative_block(value.as_str())?;
            }
            TokenType::Ident(value) => {
                self.tokenizer.reconsume();
                (a, b) = self.do_plus_block(value.as_str())?;
            }
            TokenType::Delim('+') if self.tokenizer.lookahead(1).is_ident() => {
                let value = self.consume_any_ident()?;
                (a, b) = self.do_plus_block(value.as_str())?;
            }
            TokenType::Dimension { value, unit } => {
                (a, b) = self.do_dimension_block(value, unit)?;
            }
            _ => {
                self.tokenizer.reconsume();
                return Err(Error::new(
                    "Expected anplusb".to_string(),
                    self.tokenizer.current_location(),
                ));
            }
        }

        // Remove the leading + sign
        if a.starts_with('+') {
            a = a[1..].to_string();
        }
        if b.starts_with('+') {
            b = b[1..].to_string();
        }

        Ok(Node::new(NodeType::AnPlusB { a, b }, loc))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use gosub_shared::byte_stream::{ByteStream, Encoding};

    macro_rules! test {
        ($func:ident, $input:expr, $expected:expr) => {
            let mut stream = ByteStream::new(Encoding::UTF8, None);
            stream.read_from_str($input, Some(Encoding::UTF8));
            stream.close();

            let mut parser = crate::Css3::new(&mut stream);
            let result = parser.$func().unwrap();

            assert_eq!(result.node_type, $expected);
        };
    }

    #[test]
    fn anplusb() {
        test!(
            parse_anplusb,
            "1n+2",
            Box::new(NodeType::AnPlusB {
                a: "1".to_string(),
                b: "2".to_string()
            })
        );
        test!(
            parse_anplusb,
            "1n-2",
            Box::new(NodeType::AnPlusB {
                a: "1".to_string(),
                b: "-2".to_string()
            })
        );
        test!(
            parse_anplusb,
            "-1n+2",
            Box::new(NodeType::AnPlusB {
                a: "-1".to_string(),
                b: "2".to_string()
            })
        );
        test!(
            parse_anplusb,
            "-1n-20",
            Box::new(NodeType::AnPlusB {
                a: "-1".to_string(),
                b: "-20".to_string()
            })
        );
        test!(
            parse_anplusb,
            "-1n+20",
            Box::new(NodeType::AnPlusB {
                a: "-1".to_string(),
                b: "20".to_string()
            })
        );
        test!(
            parse_anplusb,
            "1n",
            Box::new(NodeType::AnPlusB {
                a: "1".to_string(),
                b: "0".to_string()
            })
        );
        test!(
            parse_anplusb,
            "10n-5",
            Box::new(NodeType::AnPlusB {
                a: "10".to_string(),
                b: "-5".to_string()
            })
        );
        test!(
            parse_anplusb,
            "0n+5",
            Box::new(NodeType::AnPlusB {
                a: "0".to_string(),
                b: "5".to_string()
            })
        );
        test!(
            parse_anplusb,
            "1n+0",
            Box::new(NodeType::AnPlusB {
                a: "1".to_string(),
                b: "0".to_string()
            })
        );
        test!(
            parse_anplusb,
            "n+0",
            Box::new(NodeType::AnPlusB {
                a: "1".to_string(),
                b: "0".to_string()
            })
        );
        test!(
            parse_anplusb,
            "n",
            Box::new(NodeType::AnPlusB {
                a: "1".to_string(),
                b: "0".to_string()
            })
        );
        test!(
            parse_anplusb,
            "2n+0",
            Box::new(NodeType::AnPlusB {
                a: "2".to_string(),
                b: "0".to_string()
            })
        );
        test!(
            parse_anplusb,
            "2n",
            Box::new(NodeType::AnPlusB {
                a: "2".to_string(),
                b: "0".to_string()
            })
        );
        test!(
            parse_anplusb,
            "3n-6",
            Box::new(NodeType::AnPlusB {
                a: "3".to_string(),
                b: "-6".to_string()
            })
        );
        test!(
            parse_anplusb,
            "3n + 1",
            Box::new(NodeType::AnPlusB {
                a: "3".to_string(),
                b: "1".to_string()
            })
        );
        test!(
            parse_anplusb,
            "+3n - 2",
            Box::new(NodeType::AnPlusB {
                a: "3".to_string(),
                b: "-2".to_string()
            })
        );
        test!(
            parse_anplusb,
            "-n+ 6",
            Box::new(NodeType::AnPlusB {
                a: "-1".to_string(),
                b: "6".to_string()
            })
        );
        test!(
            parse_anplusb,
            "-n+6",
            Box::new(NodeType::AnPlusB {
                a: "-1".to_string(),
                b: "6".to_string()
            })
        );
        test!(
            parse_anplusb,
            "-n +6",
            Box::new(NodeType::AnPlusB {
                a: "-1".to_string(),
                b: "6".to_string()
            })
        );
    }
}
