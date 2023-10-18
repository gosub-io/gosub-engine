use crate::css3::tokens::{Token, TOKEN_REFS};
use regex::{self, Regex};

/// CSS Tokenizer
#[derive(Debug, Default, PartialEq)]
pub struct Tokenizer {
    pub cursor: usize,
    raw: String,
}

impl Tokenizer {
    pub fn new() -> Self {
        Tokenizer {
            cursor: 0,
            raw: String::new(),
        }
    }

    pub fn init(&mut self, raw: &str) {
        self.raw = raw.to_string();
        self.cursor = 0;
    }

    pub fn has_more_tokens(&self) -> bool {
        self.cursor < self.raw.len()
    }

    pub fn is_eof(&self) -> bool {
        self.cursor == self.raw.len()
    }

    pub fn get_next_token(&mut self) -> Option<Token> {
        if !self.has_more_tokens() {
            return None;
        }

        let raw = &self.raw[self.cursor..];
        for (regex, token_type) in TOKEN_REFS.iter() {
            let re = Regex::new(regex).unwrap();
            let result = re.captures(raw);

            // println!(
            //     "[get_next_token] value={:?}, token_type={:?} for raw={:?}",
            //     result,
            //     token_type,
            //     raw.lines().collect::<Vec<&str>>().first(),
            // );

            if let Some(cap) = result {
                let value = cap.get(0).unwrap().as_str();
                self.cursor += value.len();

                if token_type.is_none() {
                    return self.get_next_token();
                }

                return Some(Token::new(token_type.unwrap(), value.to_string()));
            }
        }

        None
    }
}

#[cfg(test)]
mod test {
    use crate::css3::tokens::TokenType;

    use super::*;

    macro_rules! assert_next_token {
        ($self:expr, $token_type:expr, $token_value:expr) => {
            let token = $self.get_next_token();

            if token.is_none() {
                assert_eq!($token_type.is_none(), true);
            } else {
                assert_eq!(
                    token.unwrap(),
                    Token {
                        token_type: $token_type.unwrap(),
                        value: $token_value.unwrap().to_string(),
                    },
                )
            }
        };
    }

    #[test]
    fn match_macro_tokens() {
        let mut tokenizer = Tokenizer::default();
        tokenizer.init("123 -ident-test-1");

        assert!(!tokenizer.is_eof());
        assert!(tokenizer.has_more_tokens());

        assert_next_token!(tokenizer, Some(TokenType::Number), Some("123"));
        assert_next_token!(tokenizer, Some(TokenType::WhiteSpace), Some(" "));
        assert_next_token!(tokenizer, Some(TokenType::Ident), Some("-ident-test-1"));

        assert!(tokenizer.is_eof());
        assert!(!tokenizer.has_more_tokens());
    }
}
