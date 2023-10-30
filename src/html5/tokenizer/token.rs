use crate::html5::tokenizer::CHAR_NUL;
use std::collections::HashMap;

/// The different tokens types that can be emitted by the tokenizer
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

/// The different token structures that can be emitted by the tokenizer
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    DocType {
        name: Option<String>,
        force_quirks: bool,
        pub_identifier: Option<String>,
        sys_identifier: Option<String>,
    },
    StartTag {
        name: String,
        is_self_closing: bool,
        attributes: HashMap<String, String>,
    },
    EndTag {
        name: String,
        is_self_closing: bool,
    },
    Comment(String),
    Text(String),
    Eof,
}

impl Token {
    /// Returns true when any of the characters in the token are null
    pub fn is_null(&self) -> bool {
        if let Token::Text(value) = self {
            value.chars().any(|ch| ch == CHAR_NUL)
        } else {
            false
        }
    }

    /// Returns true when the token is an EOF token
    pub fn is_eof(&self) -> bool {
        matches!(self, Token::Eof)
    }

    /// Returns true if the text token is empty or only contains whitespace
    pub fn is_empty_or_white(&self) -> bool {
        if let Token::Text(value) = self {
            ["\u{0009}", "\u{000a}", "\u{000c}", "\u{000d}", "\u{0020}"].contains(&value.as_str())
        } else {
            false
        }
    }

    pub(crate) fn is_start_tag(&self, wanted_name: &str) -> bool {
        if let Token::StartTag { name, .. } = self {
            name == wanted_name
        } else {
            false
        }
    }

    pub(crate) fn is_any_start_tag(&self) -> bool {
        matches!(self, Token::StartTag { .. })
    }

    pub(crate) fn is_text_token(&self) -> bool {
        matches!(self, Token::Text(..))
    }
}

// Each token can be displayed as a string
impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Token::DocType {
                name,
                pub_identifier,
                sys_identifier,
                ..
            } => {
                let mut result = format!("<!DOCTYPE {}", name.clone().unwrap_or("".to_string()));
                if let Some(pub_id) = pub_identifier {
                    result.push_str(&format!(" PUBLIC \"{}\"", pub_id));
                }
                if let Some(sys_id) = sys_identifier {
                    result.push_str(&format!(" SYSTEM \"{}\"", sys_id));
                }
                result.push_str(" />");
                write!(f, "{}", result)
            }
            Token::Comment(value) => write!(f, "<!-- {} -->", value),
            Token::Text(value) => write!(f, "{}", value),
            Token::StartTag {
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
                write!(f, "{}", result)
            }
            Token::EndTag {
                name,
                is_self_closing,
                ..
            } => write!(f, "</{}{}>", name, if *is_self_closing { "/" } else { "" }),
            Token::Eof => write!(f, "EOF"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_is_null() {
        let token = Token::Text("Hello\0World".to_string());
        assert!(token.is_null());
    }

    #[test]
    fn test_token_is_eof() {
        let token = Token::Eof;
        assert!(token.is_eof());
    }

    #[test]
    fn test_token_is_empty_or_white() {
        let token = Token::Text(" ".to_string());
        assert!(token.is_empty_or_white());
    }

    #[test]
    fn test_token_display() {
        let token = Token::DocType {
            name: Some("html".to_string()),
            force_quirks: false,
            pub_identifier: None,
            sys_identifier: None,
        };
        assert_eq!(format!("{}", token), "<!DOCTYPE html />");

        let token = Token::DocType {
            name: Some("html".to_string()),
            force_quirks: false,
            pub_identifier: Some("foo".to_string()),
            sys_identifier: Some("bar".to_string()),
        };
        assert_eq!(
            format!("{}", token),
            "<!DOCTYPE html PUBLIC \"foo\" SYSTEM \"bar\" />"
        );
    }

    #[test]
    fn test_token_display_comment() {
        let token = Token::Comment("Hello World".to_string());
        assert_eq!(format!("{}", token), "<!-- Hello World -->");
    }

    #[test]
    fn test_token_display_comment_with_html() {
        let token = Token::Comment("<p>Hello world</p>".to_string());
        assert_eq!(format!("{}", token), "<!-- <p>Hello world</p> -->");
    }

    #[test]
    fn test_token_display_text() {
        let token = Token::Text("Hello World".to_string());
        assert_eq!(format!("{}", token), "Hello World");
    }

    #[test]
    fn test_token_display_start_tag() {
        let token = Token::StartTag {
            name: "html".to_string(),
            is_self_closing: false,
            attributes: HashMap::new(),
        };
        assert_eq!(format!("{}", token), "<html>");

        let mut attributes = HashMap::new();
        attributes.insert("foo".to_string(), "bar".to_string());

        let token = Token::StartTag {
            name: "html".to_string(),
            is_self_closing: false,
            attributes,
        };
        assert_eq!(format!("{}", token), "<html foo=\"bar\">");

        let token = Token::StartTag {
            name: "br".to_string(),
            is_self_closing: true,
            attributes: HashMap::new(),
        };
        assert_eq!(format!("{}", token), "<br />");
    }

    #[test]
    fn test_token_display_end_tag() {
        let token = Token::EndTag {
            name: "html".to_string(),
            is_self_closing: false,
        };
        assert_eq!(format!("{}", token), "</html>");
    }

    #[test]
    fn test_token_display_eof() {
        let token = Token::Eof;
        assert_eq!(format!("{}", token), "EOF");
    }
}
