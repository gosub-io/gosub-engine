use lazy_static::lazy_static;
use regex::{Captures, Regex};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::{cell::RefCell, rc::Rc};
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::html5_parser::tokenizer::token::{Attribute, Token, TokenType};
use crate::html5_parser::{
    error_logger::ErrorLogger,
    input_stream::InputStream,
    tokenizer::{
        state::State as TokenState,
        {Options, Tokenizer},
    },
};

pub const FIXTURE_ROOT: &str = "./tests/data/html5lib-tests";

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    pub tests: Vec<Test>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    pub code: String,
    pub line: i64,
    pub col: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Test {
    pub description: String,
    pub input: String,
    pub output: Vec<Vec<Value>>,
    #[serde(default)]
    pub errors: Vec<Error>,
    #[serde(default)]
    pub double_escaped: Option<bool>,
    #[serde(default)]
    pub initial_states: Vec<String>,
    pub last_start_tag: Option<String>,
}

pub struct TokenizerBuilder {
    input_stream: InputStream,
    state: TokenState,
    last_start_tag: Option<String>,
}

impl TokenizerBuilder {
    pub fn build(&mut self) -> Tokenizer<'_> {
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
        Tokenizer::new(
            &mut self.input_stream,
            Some(Options {
                initial_state: self.state,
                last_start_tag: self.last_start_tag.to_owned().unwrap_or(String::from("")),
            }),
            error_logger.clone(),
        )
    }
}

impl Test {
    pub fn builders(&self) -> Vec<TokenizerBuilder> {
        let mut builders = vec![];

        // If no initial state is given, assume Data state
        let mut states = self.initial_states.clone();
        if states.is_empty() {
            states.push(String::from("Data state"));
        }

        for state in states.iter() {
            let state = match state.as_str() {
                "PLAINTEXT state" => TokenState::PlaintextState,
                "RAWTEXT state" => TokenState::RawTextState,
                "RCDATA state" => TokenState::RcDataState,
                "Script data state" => TokenState::ScriptDataState,
                "CDATA section state" => TokenState::CDataSectionState,
                "Data state" => TokenState::DataState,
                _ => panic!("unknown state found in test: {} ", state),
            };

            let mut is = InputStream::new();
            let input = if self.double_escaped.unwrap_or(false) {
                from_utf16_lossy(self.input.as_str())
            } else {
                self.input.to_string()
            };
            is.read_from_str(input.as_str(), None);

            let builder = TokenizerBuilder {
                input_stream: is,
                last_start_tag: self.last_start_tag.clone(),
                state,
            };

            builders.push(builder);
        }

        builders
    }

    pub fn assert_valid(&self) {
        for mut builder in self.builders() {
            let mut tokenizer = builder.build();

            // If there is no output, still do an (initial) next token so the parser can generate
            // errors.
            if self.output.is_empty() {
                tokenizer.next_token();
            }

            // There can be multiple tokens to match. Make sure we match all of them
            for expected_token in self.output.iter() {
                let t = tokenizer.next_token();
                self.assert_token(t, expected_token);
            }

            let borrowed_error_logger = tokenizer.error_logger.borrow();
            assert_eq!(borrowed_error_logger.get_errors().len(), self.errors.len());

            // Check error messages
            for error in &self.errors {
                self.assert_error(&tokenizer, error);
            }
        }
    }

    // Run through the parsing without making assertions, for use in benchmarking and in order to
    // disclose any panics that might happen
    pub fn tokenize(&self) {
        for mut builder in self.builders() {
            let mut tokenizer = builder.build();

            for _ in self.output.iter() {
                tokenizer.next_token();
            }
        }
    }

    fn assert_error(&self, tokenizer: &Tokenizer, expected: &Error) {
        // Iterate all generated errors to see if we have an exact match
        for actual in tokenizer.get_error_logger().get_errors() {
            if actual.message == expected.code
                && actual.line as i64 == expected.line
                && actual.col as i64 == expected.col
            {
                return;
            }
        }

        // Try and find an error that matches the code, but has a different line/pos. Even though
        // it's not always correct, it might be a off-by-one position.
        for actual in tokenizer.get_error_logger().get_errors() {
            if actual.message == expected.code
                && (actual.line as i64 != expected.line || actual.col as i64 != expected.col)
            {
                panic!(
                    "[{}]: wanted {:?}, got {:?}",
                    self.description, expected, actual
                );
            }
        }

        panic!(
            "expected error '{}' at {}:{}",
            expected.code, expected.line, expected.col
        );
    }

    fn assert_token(&self, have: Token, expected: &[Value]) {
        use crate::html5_parser::tokenizer::token::TokenTrait;

        let double_escaped = self.double_escaped.unwrap_or(false);

        let tp = expected.get(0).unwrap();

        let expected_token_type = match tp.as_str().unwrap() {
            "DOCTYPE" => TokenType::DocTypeToken,
            "StartTag" => TokenType::StartTagToken,
            "EndTag" => TokenType::EndTagToken,
            "Comment" => TokenType::CommentToken,
            "Character" => TokenType::TextToken,
            _ => panic!("unknown output token type {:?}", tp.as_str().unwrap()),
        };

        assert_eq!(
            have.type_of(),
            expected_token_type,
            "incorrect token type found (want: {:?}, got {:?})",
            expected_token_type,
            have.type_of(),
        );

        match have {
            Token::DocTypeToken {
                name,
                force_quirks,
                pub_identifier,
                sys_identifier,
            } => self.assert_doctype(expected, name, force_quirks, pub_identifier, sys_identifier),
            Token::StartTagToken {
                name,
                attributes,
                is_self_closing,
            } => self.assert_starttag(expected, &name, attributes, is_self_closing),
            Token::EndTagToken { name, .. } => self.assert_endtag(expected, &name, double_escaped),
            Token::CommentToken { value } => self.assert_comment(expected, &value, double_escaped),
            Token::TextToken { value } => self.assert_text(expected, &value, double_escaped),
            Token::EofToken => panic!("expected eof token"),
        }
    }

    fn assert_starttag(
        &self,
        expected: &[Value],
        name: &str,
        attributes: HashMap<String, String>,
        is_self_closing: bool,
    ) {
        let expected_name = expected.get(1).and_then(|v| v.as_str()).unwrap();
        let expected_attrs = expected.get(2).and_then(|v| v.as_object());
        let expected_self_closing = expected.get(3).and_then(|v| v.as_bool());

        assert_eq!(expected_name, name, "incorrect start tag");

        if let Some(expected_self_closing) = expected_self_closing {
            assert_eq!(
                expected_self_closing, is_self_closing,
                "incorrect start tag (expected self-closing)"
            );
        }

        if expected_attrs.is_none() && attributes.is_empty() {
            // No attributes to check
            return;
        }

        // Convert the expected attr to Vec<(string, string)>
        let expected_attrs: Vec<Attribute> = expected_attrs.map_or(Vec::new(), |map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|v| Attribute {
                        name: key.clone(),
                        value: v.to_string(),
                    })
                })
                .collect()
        });

        let attributes: Vec<Attribute> = attributes
            .iter()
            .map(|(key, value)| Attribute {
                name: key.clone(),
                value: value.clone(),
            })
            .collect();

        let set1: HashSet<_> = expected_attrs.iter().collect();
        let set2: HashSet<_> = attributes.iter().collect();

        assert_eq!(set1, set2, "attribute mismatch");
    }

    fn assert_comment(&self, expected: &[Value], value: &str, is_double_escaped: bool) {
        let output_ref = expected.get(1).unwrap().as_str().unwrap();
        let output = if is_double_escaped {
            from_utf16_lossy(output_ref)
        } else {
            output_ref.to_string()
        };

        assert_eq!(value, output, "incorrect text found in comment token");
    }

    fn assert_text(&self, expected: &[Value], value: &str, is_double_escaped: bool) {
        let output_ref = expected.get(1).unwrap().as_str().unwrap();
        let output = if is_double_escaped {
            from_utf16_lossy(output_ref)
        } else {
            output_ref.to_string()
        };

        assert_eq!(value, output, "incorrect text found in text token",);
    }

    fn assert_endtag(&self, expected: &[Value], name: &str, is_double_escaped: bool) {
        let output_ref = expected.get(1).unwrap().as_str().unwrap();
        let output = if is_double_escaped {
            from_utf16_lossy(output_ref)
        } else {
            output_ref.to_string()
        };

        assert_eq!(name, output, "incorrect end tag");
    }

    // Check if a given doctype matches the expected result
    fn assert_doctype(
        &self,
        expected: &[Value],
        name: Option<String>,
        force_quirks: bool,
        pub_identifier: Option<String>,
        sys_identifier: Option<String>,
    ) {
        let expected_name = expected[1].as_str();
        let expected_pub = expected[2].as_str();
        let expected_sys = expected[3].as_str();
        let expected_quirk = expected[4].as_bool();

        assert_eq!(expected_name, name.as_deref(), "incorrect doctype");

        if let Some(expected_quirk) = expected_quirk {
            assert_ne!(
                expected_quirk, force_quirks,
                "incorrect doctype (wanted quirk: {})",
                expected_quirk
            );
        }

        assert_eq!(
            expected_pub,
            pub_identifier.as_deref(),
            "incorrect doctype (wanted pub id: '{:?}', got '{:?}')",
            expected_pub,
            pub_identifier,
        );

        assert_eq!(
            expected_sys,
            sys_identifier.as_deref(),
            "incorrect doctype (wanted sys id: '{:?}', got '{:?}')",
            expected_sys,
            sys_identifier
        );
    }
}

pub fn from_utf16_lossy(input: &str) -> String {
    // TODO: Maybe use String::from_utf8_lossy(input.as_bytes()).into() instead
    // https://doc.rust-lang.org/std/string/struct.String.html#method.from_utf8_lossy

    lazy_static! {
        static ref RE: Regex = Regex::new(r"\\u([0-9a-fA-F]{4})").unwrap();
    }

    RE.replace_all(input, |cap: &Captures| {
        let n = u16::from_str_radix(&cap[1], 16).unwrap();
        // There are UTF-16 characters that the following will not decode into UTF-8, so we might
        // be dropping characters when a DecodeUtf16Error error is encountered.
        std::char::decode_utf16([n])
            .filter_map(|r| r.ok())
            .collect::<String>()
    })
    .to_string()
}

pub fn fixture_from_filename(filename: &str) -> Result<Root, serde_json::Error> {
    let path = PathBuf::from(FIXTURE_ROOT).join("tokenizer").join(filename);
    fixture_from_path(&path)
}

pub fn fixture_from_path<P>(path: &P) -> Result<Root, serde_json::Error>
where
    P: AsRef<Path>,
{
    let contents = fs::read_to_string(path).unwrap();
    // TODO: use thiserror to translate library errors
    serde_json::from_str(&contents)
}

pub fn fixtures() -> impl Iterator<Item = Root> {
    let root = PathBuf::from(FIXTURE_ROOT).join("tokenizer");
    fs::read_dir(root).unwrap().flat_map(|entry| {
        let path = format!("{}", entry.unwrap().path().display());
        fixture_from_path(&path).ok()
    })
}
