use gosub_engine::html5_parser::error_logger::ErrorLogger;
use regex::Regex;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use test_case::test_case;

use gosub_engine::html5_parser::input_stream::InputStream;
use gosub_engine::html5_parser::tokenizer::state::State as TokenState;
use gosub_engine::html5_parser::tokenizer::token::{Attribute, Token, TokenTrait, TokenType};
use gosub_engine::html5_parser::tokenizer::{Options, Tokenizer};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    pub tests: Vec<Test>,
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

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    pub code: String,
    pub line: i64,
    pub col: i64,
}

fn assert_tokenization(test: &Test) {
    // If no initial state is given, assume Data state
    let mut states = test.initial_states.clone();
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
        let input = if test.double_escaped.unwrap_or(false) {
            escape(test.input.as_str())
        } else {
            test.input.to_string()
        };
        is.read_from_str(input.as_str(), None);

        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
        let mut tokenizer = Tokenizer::new(
            &mut is,
            Some(Options {
                initial_state: state,
                last_start_tag: test.last_start_tag.clone().unwrap_or(String::from("")),
            }),
            error_logger.clone(),
        );

        // If there is no output, still do an (initial) next token so the parser can generate
        // errors.
        if test.output.is_empty() {
            tokenizer.next_token();
        }

        // There can be multiple tokens to match. Make sure we match all of them
        for expected_token in test.output.iter() {
            let t = tokenizer.next_token();
            assert_token(t, expected_token, test.double_escaped.unwrap_or(false));
        }

        let borrowed_error_logger = error_logger.borrow();
        assert_eq!(borrowed_error_logger.get_errors().len(), test.errors.len());

        // Check error messages
        for error in &test.errors {
            assert_error(&tokenizer, error);
        }
    }
}

fn assert_error(tokenizer: &Tokenizer, expected_err: &Error) {
    // Iterate all generated errors to see if we have an exact match
    for got_err in tokenizer.get_error_logger().get_errors() {
        if got_err.message == expected_err.code
            && got_err.line as i64 == expected_err.line
            && got_err.col as i64 == expected_err.col
        {
            return;
        }
    }

    // Try and find an error that matches the code, but has a different line/pos. Even though
    // it's not always correct, it might be a off-by-one position.
    for got_err in tokenizer.get_error_logger().get_errors() {
        if got_err.message == expected_err.code
            && (got_err.line as i64 != expected_err.line || got_err.col as i64 != expected_err.col)
        {
            panic!(
                "expected error '{}' at {}:{}",
                expected_err.code, expected_err.line, expected_err.col
            );
        }
    }

    panic!(
        "expected error '{}' at {}:{}",
        expected_err.code, expected_err.line, expected_err.col
    );
}

fn assert_token(have: Token, expected: &[Value], double_escaped: bool) {
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
        } => assert_doctype(expected, name, force_quirks, pub_identifier, sys_identifier),
        Token::StartTagToken {
            name,
            attributes,
            is_self_closing,
        } => assert_starttag(expected, name, attributes, is_self_closing),
        Token::EndTagToken { name, .. } => assert_endtag(expected, name, double_escaped),
        Token::CommentToken { value } => assert_comment(expected, value, double_escaped),
        Token::TextToken { value } => assert_text(expected, value, double_escaped),
        Token::EofToken => panic!("expected eof token"),
    }
}

fn assert_starttag(
    expected: &[Value],
    name: String,
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

fn assert_comment(expected: &[Value], value: String, is_double_escaped: bool) {
    let output_ref = expected.get(1).unwrap().as_str().unwrap();
    let output = if is_double_escaped {
        escape(output_ref)
    } else {
        output_ref.to_string()
    };

    assert_eq!(value, output, "incorrect text found in comment token");
}

fn assert_text(expected: &[Value], value: String, is_double_escaped: bool) {
    let output_ref = expected.get(1).unwrap().as_str().unwrap();
    let output = if is_double_escaped {
        escape(output_ref)
    } else {
        output_ref.to_string()
    };

    assert_eq!(value, output, "incorrect text found in text token",);
}

fn assert_endtag(expected: &[Value], name: String, is_double_escaped: bool) {
    let output_ref = expected.get(1).unwrap().as_str().unwrap();
    let output = if is_double_escaped {
        escape(output_ref)
    } else {
        output_ref.to_string()
    };

    assert_eq!(name, output, "incorrect end tag");
}

// Check if a given doctype matches the expected result
fn assert_doctype(
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

    match (expected_name, &name) {
        (Some(_), Some(_)) | (None, Some(_)) | (Some(_), None) => {
            assert_eq!(expected_name, name.as_deref(), "incorrect doctype");
        }

        (None, None) => {}
    }

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

fn escape(input: &str) -> String {
    let re = Regex::new(r"\\u([0-9a-fA-F]{4})").unwrap();
    re.replace_all(input, |caps: &regex::Captures| {
        let hex_val = u32::from_str_radix(&caps[1], 16).unwrap();

        // This will also convert surrogates?
        unsafe { char::from_u32_unchecked(hex_val).to_string() }
    })
    .into_owned()
}

#[test_case("contentModelFlags.test")]
#[test_case("domjs.test")]
#[test_case("entities.test")]
#[test_case("escapeFlag.test")]
#[test_case("namedEntities.test")]
#[test_case("numericEntities.test")]
#[test_case("pendingSpecChanges.test")]
#[test_case("test1.test")]
#[test_case("test2.test")]
// #[test_case("test3.test")]
#[test_case("test4.test")]
// #[test_case("unicodeCharsProblematic.test")]
#[test_case("unicodeChars.test")]
// #[test_case("xmlViolation.test")]
fn tokenization(filename: &str) {
    const ROOT: &str = "./tests/data/html5lib-tests/tokenizer";
    let path = PathBuf::from(ROOT).join(filename);
    let contents = fs::read_to_string(&path).unwrap();
    let container: Root = serde_json::from_str(&contents).unwrap();

    for test in container.tests {
        assert_tokenization(&test)
    }
}
