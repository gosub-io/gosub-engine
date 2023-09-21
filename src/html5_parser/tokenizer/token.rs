use std::collections::HashMap;
use crate::html5_parser::tokenizer::CHAR_NUL;

// The different tokens types that can be emitted by the tokenizer
#[derive(Debug, PartialEq)]
pub enum TokenType {
    DocTypeToken,
    StartTagToken,
    EndTagToken,
    CommentToken,
    TextToken,
    EofToken,
}

#[derive(Debug, PartialEq, Clone, Hash, Eq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

// The different token structures that can be emitted by the tokenizer
#[derive(Clone, PartialEq)]
pub enum Token {
    DocTypeToken {
        name: Option<String>,
        force_quirks: bool,
        pub_identifier: Option<String>,
        sys_identifier: Option<String>,
    },
    StartTagToken {
        name: String,
        is_self_closing: bool,
        attributes: HashMap<String, String>
    },
    EndTagToken {
        name: String,
        is_self_closing: bool,
        attributes: HashMap<String, String>
    },
    CommentToken {
        value: String,
    },
    TextToken {
        value: String,
    },
    EofToken,
}

impl Token {
    // Returns true when any of the characters in the token are null
    pub fn is_null(&self) -> bool {
        if let Token::TextToken { value } = self {
            value.chars().any(|ch| ch == CHAR_NUL)
        } else {
            false
        }
    }

    // Returns true when the token is an EOF token
    pub fn is_eof(&self) -> bool {
        if let Token::EofToken = self {
            true
        } else {
            false
        }
    }

    // Returns true if the text token is empty or only contains whitespace
    pub fn is_empty_or_white(&self) -> bool {
        if let Token::TextToken { value } = self {
            value.trim().is_empty()
        } else {
            false
        }
    }
}

// Each token can be displayed as a string
impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Token::DocTypeToken {
                name,
                force_quirks,
                pub_identifier,
                sys_identifier,
            } => {
                let mut result = format!("<!DOCTYPE {}", name.clone().unwrap_or("".to_string()));
                if *force_quirks {
                    result.push_str(" FORCE_QUIRKS!");
                }
                if let Some(pub_id) = pub_identifier {
                    result.push_str(&format!(" {}", pub_id));
                }
                if let Some(sys_id) = sys_identifier {
                    result.push_str(&format!(" {}", sys_id));
                }
                result.push_str(" />");
                write!(f, "{}", result)
            }
            Token::CommentToken { value } => write!(f, "Comment[<!-- {} -->]", value),
            Token::TextToken { value } => write!(f, "Text[{}]", value),
            Token::StartTagToken {
                name,
                is_self_closing,
                attributes,
            } => {
                let mut result = format!("<{}", name);
                for (key, value) in attributes.iter() {
                    result.push_str(&format!(" {}=\"{}\"", key, value));
                }
                if *is_self_closing {
                    result.push_str(" /");
                }
                result.push('>');
                write!(f, "StartTag[{}]", result)
            }
            Token::EndTagToken { name, is_self_closing, .. } => write!(f, "EndTag[</{}{}>]", name, if *is_self_closing { "/" } else { "" }),
            Token::EofToken => write!(f, "EOF"),
        }
    }
}

pub trait TokenTrait {
    // Return the token type of the given token
    fn type_of(&self) -> TokenType;
}

// Each token implements the TokenTrait and has a type_of that will return the tokentype.
impl TokenTrait for Token {
    fn type_of(&self) -> TokenType {
        match self {
            Token::DocTypeToken { .. } => TokenType::DocTypeToken,
            Token::StartTagToken { .. } => TokenType::StartTagToken,
            Token::EndTagToken { .. } => TokenType::EndTagToken,
            Token::CommentToken { .. } => TokenType::CommentToken,
            Token::TextToken { .. } => TokenType::TextToken,
            Token::EofToken => TokenType::EofToken,
        }
    }
}

#[cfg(test)]

mod tests {
    use super::*;

    #[test]
    fn test_token_type() {
        let token = Token::DocTypeToken {
            name: None,
            force_quirks: false,
            pub_identifier: None,
            sys_identifier: None,
        };
        assert_eq!(token.type_of(), TokenType::DocTypeToken);
    }

    #[test]
    fn test_token_is_null() {
        let token = Token::TextToken {
            value: "Hello\0World".to_string(),
        };
        assert!(token.is_null());
    }

    #[test]
    fn test_token_is_eof() {
        let token = Token::EofToken;
        assert!(token.is_eof());
    }

    #[test]
    fn test_token_is_empty_or_white() {
        let token = Token::TextToken {
            value: "   ".to_string(),
        };
        assert!(token.is_empty_or_white());
    }

    #[test]
    fn test_token_display() {
        let token = Token::DocTypeToken {
            name: Some("html".to_string()),
            force_quirks: false,
            pub_identifier: None,
            sys_identifier: None,
        };
        assert_eq!(format!("{}", token), "<!DOCTYPE html />");

        let token = Token::DocTypeToken {
            name: Some("html".to_string()),
            force_quirks: false,
            pub_identifier: Some("foo".to_string()),
            sys_identifier: Some("bar".to_string()),
        };
        assert_eq!(format!("{}", token), "<!DOCTYPE html />");

    }

    #[test]
    fn test_token_display_comment() {
        let token = Token::CommentToken {
            value: "Hello World".to_string(),
        };
        assert_eq!(format!("{}", token), "Comment[<!-- Hello World -->]");
    }

    #[test]
    fn test_token_display_text() {
        let token = Token::TextToken {
            value: "Hello World".to_string(),
        };
        assert_eq!(format!("{}", token), "Text[Hello World]");
    }

    #[test]
    fn test_token_display_start_tag() {
        let token = Token::StartTagToken {
            name: "html".to_string(),
            is_self_closing: false,
            attributes: HashMap::new(),
        };
        assert_eq!(format!("{}", token), "StartTag[<html>]");

        let mut attributes = HashMap::new();
        attributes.insert("foo".to_string(), "bar".to_string());

        let token = Token::StartTagToken {
            name: "html".to_string(),
            is_self_closing: false,
            attributes,
        };
        assert_eq!(format!("{}", token), "StartTag[<html foo=\"bar\">]");

        let token = Token::StartTagToken {
            name: "br".to_string(),
            is_self_closing: true,
            attributes: HashMap::new(),
        };
        assert_eq!(format!("{}", token), "StartTag[<br/>]");
    }

    #[test]
    fn test_token_display_end_tag() {
        let token = Token::EndTagToken {
            name: "html".to_string(),
            is_self_closing: false,
            attributes: HashMap::new(),
        };
        assert_eq!(format!("{}", token), "EndTag[</html>]");
    }

    #[test]
    fn test_token_display_eof() {
        let token = Token::EofToken;
        assert_eq!(format!("{}", token), "EOF");
    }
}