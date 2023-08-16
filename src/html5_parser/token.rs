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

pub enum Token {
    DocTypeToken {
        name: String,
        force_quirks: bool,
        pub_identifier: Option<String>,
        sys_identifier: Option<String>,
    },
    StartTagToken {
        name: String,
        is_self_closing: bool,
        attributes: Vec<(String, String)>,
    },
    EndTagToken {
        name: String,
    },
    CommentToken {
        value: String,
    },
    TextToken {
        value: String,
    },
    EofToken,
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Token::DocTypeToken {
                name,
                force_quirks,
                pub_identifier,
                sys_identifier,
            } => {
                let mut result = format!("<{} ", name);
                if *force_quirks {
                    result.push_str(" FORCE_QUIRKS!");
                }
                if let Some(pub_id) = pub_identifier {
                    result.push_str(&format!(" {} ", pub_id));
                }
                if let Some(sys_id) = sys_identifier {
                    result.push_str(&format!(" {} ", sys_id));
                }
                result.push('>');
                write!(f, "{}", result)
            }
            Token::CommentToken { value } => write!(f, "<!--{}-->", value),
            Token::TextToken { value } => write!(f, "{}", value),
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
                write!(f, "{}", result)
            }
            Token::EndTagToken { name } => write!(f, "</{}>", name),
            Token::EofToken => write!(f, "EOF"),
        }
    }
}

pub trait TokenTrait {
    // Return the token type of the given token
    fn type_of(&self) -> TokenType;
}

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
