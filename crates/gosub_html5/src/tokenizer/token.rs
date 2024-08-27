use crate::tokenizer::CHAR_NUL;
use gosub_shared::byte_stream::Location;
use std::collections::HashMap;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
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
        location: Location,
    },
    StartTag {
        name: String,
        is_self_closing: bool,
        attributes: HashMap<String, String>,
        location: Location,
    },
    EndTag {
        name: String,
        is_self_closing: bool,
        location: Location,
    },
    Comment {
        comment: String,
        location: Location,
    },
    Text {
        text: String,
        location: Location,
    },
    Eof {
        location: Location,
    },
}

impl Token {
    /// Returns true when there is a mixture of white and non-white and \0 characters in the token
    pub(crate) fn is_mixed(&self) -> bool {
        // Check if there are white characters AND non-white characters in the token
        if let Token::Text { text: value, .. } = self {
            let mut found = 0;

            if value.chars().any(|ch| ch.is_ascii_whitespace()) {
                found += 1;
            }
            if value.chars().any(|ch| ch == '\0') {
                found += 1;
            }
            if value
                .chars()
                .any(|ch| !ch.is_ascii_whitespace() && ch != '\0')
            {
                found += 1;
            }
            found > 1
        } else {
            false
        }
    }

    /// Returns true when there is a mixture of \0 and non-\0 characters in the token
    pub(crate) fn is_mixed_null(&self) -> bool {
        // Check if there are white characters AND non-white characters in the token
        if let Token::Text { text: value, .. } = self {
            value.chars().any(|ch| ch == '\0') && value.chars().any(|ch| ch != '\0')
        } else {
            false
        }
    }

    pub fn get_location(&self) -> Location {
        match self {
            Token::DocType { location, .. } => location.clone(),
            Token::StartTag { location, .. } => location.clone(),
            Token::EndTag { location, .. } => location.clone(),
            Token::Comment { location, .. } => location.clone(),
            Token::Text { location, .. } => location.clone(),
            Token::Eof { location, .. } => location.clone(),
        }
    }

    pub fn set_location(&mut self, location: Location) {
        match self {
            Token::DocType { location: loc, .. } => loc.clone_from(&location),
            Token::StartTag { location: loc, .. } => loc.clone_from(&location),
            Token::EndTag { location: loc, .. } => loc.clone_from(&location),
            Token::Comment { location: loc, .. } => loc.clone_from(&location),
            Token::Text { location: loc, .. } => loc.clone_from(&location),
            Token::Eof { location: loc, .. } => loc.clone_from(&location),
        }
    }

    /// Returns true when any of the characters in the token are null
    pub fn is_null(&self) -> bool {
        if let Token::Text { text: value, .. } = self {
            value.chars().any(|ch| ch == CHAR_NUL)
        } else {
            false
        }
    }

    /// Returns true when the token is an EOF token
    pub fn is_eof(&self) -> bool {
        matches!(self, Token::Eof { .. })
    }

    /// Returns true if the text token is empty or only contains whitespace
    pub fn is_empty_or_white(&self) -> bool {
        if let Token::Text { text: value, .. } = self {
            if value.is_empty() {
                return true;
            }

            value.chars().all(|ch| ch.is_ascii_whitespace())
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
        matches!(self, Token::Text { .. })
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
                let mut result = format!("<!DOCTYPE {}", name.clone().unwrap_or_default());
                if let Some(pub_id) = pub_identifier {
                    result.push_str(&format!(r#" PUBLIC "{pub_id}""#));
                }
                if let Some(sys_id) = sys_identifier {
                    result.push_str(&format!(r#" SYSTEM "{sys_id}""#));
                }
                result.push_str(" />");
                write!(f, "{result}")
            }
            Token::Comment { comment: value, .. } => write!(f, "<!-- {value} -->"),
            Token::Text { text: value, .. } => write!(f, "{value}"),
            Token::StartTag {
                name,
                is_self_closing,
                attributes,
                ..
            } => {
                let mut result = format!("<{name}");
                for (key, value) in attributes {
                    result.push_str(&format!(r#" {key}="{value}""#));
                }
                if *is_self_closing {
                    result.push_str(" /");
                }
                result.push('>');
                write!(f, "{result}")
            }
            Token::EndTag {
                name,
                is_self_closing,
                ..
            } => write!(f, "</{}{}>", name, if *is_self_closing { "/" } else { "" }),
            Token::Eof { .. } => write!(f, "EOF"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_is_null() {
        let token = Token::Text {
            text: "Hello\0World".to_string(),
            location: Location::default(),
        };
        assert!(token.is_null());
    }

    #[test]
    fn test_token_is_eof() {
        let token = Token::Eof {
            location: Location::default(),
        };
        assert!(token.is_eof());
    }

    #[test]
    fn test_token_is_empty_or_white() {
        let token = Token::Text {
            text: " ".to_string(),
            location: Location::default(),
        };
        assert!(token.is_empty_or_white());
    }

    #[test]
    fn test_token_display() {
        let token = Token::DocType {
            name: Some("html".to_string()),
            force_quirks: false,
            pub_identifier: None,
            sys_identifier: None,
            location: Location::default(),
        };
        assert_eq!(format!("{token}"), "<!DOCTYPE html />");

        let token = Token::DocType {
            name: Some("html".to_string()),
            force_quirks: false,
            pub_identifier: Some("foo".to_string()),
            sys_identifier: Some("bar".to_string()),
            location: Location::default(),
        };
        assert_eq!(
            format!("{token}"),
            r#"<!DOCTYPE html PUBLIC "foo" SYSTEM "bar" />"#
        );
    }

    #[test]
    fn test_token_display_comment() {
        let token = Token::Comment {
            comment: "Hello World".to_string(),
            location: Location::default(),
        };
        assert_eq!(format!("{token}"), "<!-- Hello World -->");
    }

    #[test]
    fn test_token_display_comment_with_html() {
        let token = Token::Comment {
            comment: "<p>Hello world</p>".to_string(),
            location: Location::default(),
        };
        assert_eq!(format!("{token}"), "<!-- <p>Hello world</p> -->");
    }

    #[test]
    fn test_token_display_text() {
        let token = Token::Text {
            text: "Hello World".to_string(),
            location: Location::default(),
        };
        assert_eq!(format!("{token}"), "Hello World");
    }

    #[test]
    fn test_token_display_start_tag() {
        let token = Token::StartTag {
            name: "html".to_string(),
            is_self_closing: false,
            attributes: HashMap::new(),
            location: Location::default(),
        };
        assert_eq!(format!("{token}"), "<html>");

        let mut attributes = HashMap::new();
        attributes.insert("foo".to_string(), "bar".to_string());

        let token = Token::StartTag {
            name: "html".to_string(),
            is_self_closing: false,
            attributes,
            location: Location::default(),
        };
        assert_eq!(format!("{token}"), r#"<html foo="bar">"#);

        let token = Token::StartTag {
            name: "br".to_string(),
            is_self_closing: true,
            attributes: HashMap::new(),
            location: Location::default(),
        };
        assert_eq!(format!("{token}"), "<br />");
    }

    #[test]
    fn test_token_display_end_tag() {
        let token = Token::EndTag {
            name: "html".to_string(),
            is_self_closing: false,
            location: Location::default(),
        };
        assert_eq!(format!("{token}"), "</html>");
    }

    #[test]
    fn test_token_display_eof() {
        let token = Token::Eof {
            location: Location::default(),
        };
        assert_eq!(format!("{token}"), "EOF");
    }

    #[test]
    fn test_is_start_tag() {
        let token = Token::StartTag {
            name: "div".to_string(),
            is_self_closing: false,
            attributes: HashMap::new(),
            location: Location::default(),
        };
        assert!(token.is_start_tag("div"));
        assert!(!token.is_start_tag("span"));
    }

    #[test]
    fn test_is_any_start_tag() {
        let start_tag = Token::StartTag {
            name: "div".to_string(),
            is_self_closing: false,
            attributes: HashMap::new(),
            location: Location::default(),
        };
        let other_tag = Token::Text {
            text: "TestingText".to_string(),
            location: Location::default(),
        };
        assert!(start_tag.is_any_start_tag());
        assert!(!other_tag.is_any_start_tag());
    }

    #[test]
    fn test_is_text_token() {
        let text_token = Token::Text {
            text: "TestingText".to_string(),
            location: Location::default(),
        };
        let other_token = Token::StartTag {
            name: "div".to_string(),
            is_self_closing: false,
            attributes: HashMap::new(),
            location: Location::default(),
        };
        assert!(text_token.is_text_token());
        assert!(!other_token.is_text_token());
    }
}
