use crate::css3_parser::tokens::{Token, TOKENS_REGEXS};
use regex::{self, Regex};

/// CSS Tokenizer
#[derive(Debug, PartialEq)]
pub struct CSSTokenizer {
    pub cursor: usize,
    raw: String,
}

impl Default for CSSTokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl CSSTokenizer {
    pub fn new() -> Self {
        CSSTokenizer {
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
        for (regex, token_type) in TOKENS_REGEXS.iter() {
            let re = Regex::new(regex).unwrap();
            let result = re.captures(raw);

            println!("RegExp Result: {:#?}", result);

            if let Some(cap) = result {
                let value = cap.get(0).unwrap().as_str();
                self.cursor += value.len();

                println!(
                    "Tokenizer.get_next_token: value={}, token_type={:?} for raw={}",
                    value,
                    token_type,
                    raw.trim(),
                );

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
    use crate::css3_parser::tokens::TokenType;

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
    fn should_match_tokens() {
        let mut tokenizer = CSSTokenizer::default();
        tokenizer.init(".><,");
        assert_eq!(tokenizer.is_eof(), false);
        assert_eq!(tokenizer.has_more_tokens(), true);

        assert_next_token!(tokenizer, Some(TokenType::Dot), Some("."));
        assert_next_token!(tokenizer, Some(TokenType::GreaterThan), Some(">"));
        assert_next_token!(tokenizer, Some(TokenType::LessThan), Some("<"));
        assert_next_token!(tokenizer, Some(TokenType::Comma), Some(","));
        assert_next_token!(tokenizer, None::<TokenType>, None::<&str>);

        assert_eq!(tokenizer.is_eof(), true);
        assert_eq!(tokenizer.has_more_tokens(), false);
    }

    #[test]
    fn should_skip_spaces() {
        let mut tokenizer = CSSTokenizer::default();
        tokenizer.init("#id > .class {}");

        assert_eq!(tokenizer.is_eof(), false);
        assert_eq!(tokenizer.has_more_tokens(), true);

        assert_next_token!(tokenizer, Some(TokenType::Hash), Some("#"));
        assert_next_token!(tokenizer, Some(TokenType::Identifier), Some("id"));
        assert_next_token!(tokenizer, Some(TokenType::GreaterThan), Some(">"));
        assert_next_token!(tokenizer, Some(TokenType::Dot), Some("."));
        assert_next_token!(tokenizer, Some(TokenType::Identifier), Some("class"));

        assert_eq!(tokenizer.is_eof(), false);
        assert_eq!(tokenizer.has_more_tokens(), true);
    }

    #[test]
    fn should_handle_multiline_input() {
        let mut tokenizer = CSSTokenizer::default();

        tokenizer.init(
            r#"
            
            .header {

            }

            #nav {

            }
        
        "#,
        );

        assert_next_token!(tokenizer, Some(TokenType::Dot), Some("."));
        assert_next_token!(tokenizer, Some(TokenType::Identifier), Some("header"));
        tokenizer.get_next_token(); // {
        tokenizer.get_next_token(); // }
        assert_next_token!(tokenizer, Some(TokenType::Hash), Some("#"));
        assert_next_token!(tokenizer, Some(TokenType::Identifier), Some("nav"));
        tokenizer.get_next_token(); // {
        tokenizer.get_next_token(); // }
    }
}
