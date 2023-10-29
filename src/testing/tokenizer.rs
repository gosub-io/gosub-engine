use super::FIXTURE_ROOT;
use crate::html5::{
    error_logger::ErrorLogger,
    input_stream::InputStream,
    tokenizer::{
        state::State as TokenState,
        token::Token,
        {Options, Tokenizer},
    },
};
use crate::types::Result;
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use serde::{
    de::{Error, Unexpected},
    Deserialize, Deserializer,
};
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use std::{
    fs,
    path::{Path, PathBuf},
};

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

#[derive(Default, Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenError {
    pub code: String,
    pub line: usize,
    pub col: usize,
}

#[derive(Default, Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestSpec {
    pub description: String,
    pub input: String,
    #[serde(deserialize_with = "deserialize_output")]
    pub output: Vec<Token>,
    #[serde(default)]
    pub errors: Vec<TokenError>,
    #[serde(default)]
    pub double_escaped: bool,
    #[serde(default, deserialize_with = "deserialize_states")]
    pub initial_states: Vec<TokenState>,
    #[serde(default)]
    pub last_start_tag: Option<String>,
}

// We only care about six states that are found in the tests rather than handling deserialization
// of all possible states.
fn deserialize_states<'de, D>(deserializer: D) -> std::result::Result<Vec<TokenState>, D::Error>
where
    D: Deserializer<'de>,
{
    let values: Vec<Value> = Deserialize::deserialize(deserializer)?;
    let states = values
        .into_iter()
        .map(|value| match value.as_str() {
            Some("Data state") => TokenState::Data,
            Some("CDATA section state") => TokenState::CDATASection,
            Some("PLAINTEXT state") => TokenState::PLAINTEXT,
            Some("RAWTEXT state") => TokenState::RAWTEXT,
            Some("RCDATA state") => TokenState::RCDATA,
            Some("Script data state") => TokenState::ScriptData,
            _ => unreachable!("{}", value),
        })
        .collect::<Vec<_>>();
    Ok(states)
}

// Deserialize the contents of the test.output array into Vec<Token>
fn deserialize_output<'de, D>(deserializer: D) -> std::result::Result<Vec<Token>, D::Error>
where
    D: Deserializer<'de>,
{
    let tokens: Vec<Vec<Value>> = Deserialize::deserialize(deserializer)?;
    let mut output = vec![];

    fn attributes(value: &Value) -> HashMap<String, String> {
        value
            .as_object()
            .unwrap()
            .into_iter()
            .filter_map(|(name, value)| {
                if value.is_null() {
                    None
                } else {
                    Some((name.to_owned(), value.as_str().unwrap().to_owned()))
                }
            })
            .collect::<HashMap<String, String>>()
    }

    for values in tokens {
        let kind: &str = values[0].as_str().unwrap();

        let token = match values.len() {
            2 => match kind {
                "Character" => Token::Text(values[1].as_str().unwrap().to_owned()),
                "Comment" => Token::Comment(values[1].as_str().unwrap().to_owned()),
                "EndTag" => Token::EndTag {
                    name: values[1].as_str().unwrap().to_owned(),
                    is_self_closing: false,
                },
                _ => {
                    return Err(D::Error::invalid_value(
                        Unexpected::Str(kind),
                        &"Character, Comment or EndTag",
                    ))
                }
            },

            3 => match kind {
                "StartTag" => Token::StartTag {
                    name: values[1].as_str().unwrap().to_owned(),
                    attributes: attributes(&values[2]),
                    is_self_closing: false,
                },
                _ => return Err(D::Error::invalid_value(Unexpected::Str(kind), &"StartTag")),
            },

            4 => match kind {
                "StartTag" => Token::StartTag {
                    name: values[1].as_str().unwrap().to_owned(),
                    attributes: attributes(&values[2]),
                    is_self_closing: values[3].as_bool().unwrap_or_default(),
                },
                _ => return Err(D::Error::invalid_value(Unexpected::Str(kind), &"StartTag")),
            },

            5 => match kind {
                "DOCTYPE" => Token::DocType {
                    name: values[1].as_str().map(str::to_owned),
                    pub_identifier: values[2].as_str().map(str::to_owned),
                    sys_identifier: values[3].as_str().map(str::to_owned),
                    force_quirks: !values[4].as_bool().unwrap_or_default(),
                },
                _ => return Err(D::Error::invalid_value(Unexpected::Str(kind), &"DOCTYPE")),
            },

            _ => {
                return Err(D::Error::invalid_length(
                    values.len(),
                    &"an array of length 2, 3, 4 or 5",
                ))
            }
        };

        output.push(token);
    }

    Ok(output)
}

impl TestSpec {
    pub fn builders(&self) -> Vec<TokenizerBuilder> {
        let mut builders = vec![];

        // If no initial state is given, assume Data state
        let mut states = self.initial_states.clone();
        if states.is_empty() {
            states.push(TokenState::Data);
        }

        for state in states.into_iter() {
            let mut is = InputStream::new();
            let input = if self.double_escaped {
                from_utf16_lossy(&self.input)
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
                tokenizer.next_token().unwrap();
            }

            // There can be multiple tokens to match. Make sure we match all of them
            for expected in self.output.iter() {
                let actual = tokenizer.next_token().unwrap();
                assert_eq!(self.escape(&actual), self.escape(expected));
            }

            let borrowed_error_logger = tokenizer.error_logger.borrow();
            assert_eq!(borrowed_error_logger.get_errors().len(), self.errors.len());

            // Check error messages
            for error in &self.errors {
                self.assert_error(&tokenizer, error);
            }
        }
    }

    /// Run through the parsing without making assertions, for use in benchmarking and in order to
    /// disclose any panics that might happen
    pub fn tokenize(&self) {
        for mut builder in self.builders() {
            let mut tokenizer = builder.build();

            for _ in self.output.iter() {
                tokenizer.next_token().unwrap();
            }
        }
    }

    fn assert_error(&self, tokenizer: &Tokenizer, expected: &TokenError) {
        // Iterate all generated errors to see if we have an exact match
        for actual in tokenizer.get_error_logger().get_errors() {
            if actual.message == expected.code
                && actual.line == expected.line
                && actual.col == expected.col
            {
                return;
            }
        }

        // Try and find an error that matches the code, but has a different line/pos. Even though
        // it's not always correct, it might be a off-by-one position.
        for actual in tokenizer.get_error_logger().get_errors() {
            if actual.message == expected.code
                && (actual.line != expected.line || actual.col != expected.col)
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

    fn escape(&self, token: &Token) -> Token {
        let escape = from_utf16_lossy;

        if !self.double_escaped {
            return token.to_owned();
        }

        match token {
            Token::Comment(value) => Token::Comment(escape(value)),

            Token::DocType {
                name,
                force_quirks,
                pub_identifier,
                sys_identifier,
            } => Token::DocType {
                name: name.as_ref().map(|name| escape(name)),
                force_quirks: *force_quirks,
                pub_identifier: pub_identifier.as_ref().map(|s| s.into()),
                sys_identifier: sys_identifier.as_ref().map(|s| s.into()),
            },

            Token::EndTag {
                name,
                is_self_closing,
            } => Token::EndTag {
                name: escape(name),
                is_self_closing: *is_self_closing,
            },

            Token::Eof => Token::Eof,

            Token::StartTag {
                name,
                is_self_closing,
                attributes,
            } => Token::StartTag {
                name: escape(name),
                is_self_closing: *is_self_closing,
                attributes: attributes.clone(),
            },

            Token::Text(value) => Token::Text(escape(value)),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum FixtureFile {
    Tests {
        tests: Vec<TestSpec>,
    },

    XmlTests {
        #[serde(rename = "xmlViolationTests")]
        tests: Vec<TestSpec>,
    },
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

pub fn fixture_from_filename(filename: &str) -> Result<FixtureFile> {
    let path = PathBuf::from(FIXTURE_ROOT).join("tokenizer").join(filename);
    fixture_from_path(&path)
}

pub fn fixture_from_path<P>(path: &P) -> Result<FixtureFile>
where
    P: AsRef<Path>,
{
    let contents = fs::read_to_string(path).unwrap();
    Ok(serde_json::from_str(&contents)?)
}

pub fn fixtures() -> impl Iterator<Item = FixtureFile> {
    let root = PathBuf::from(FIXTURE_ROOT).join("tokenizer");
    fs::read_dir(root).unwrap().flat_map(|entry| {
        let path = format!("{}", entry.unwrap().path().display());
        fixture_from_path(&path).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn parse(i: &str) -> TestSpec {
        serde_json::from_str(i).expect("error parsing")
    }

    #[test]
    fn entities1_test_3() {
        let test = parse(
            r#"{
                "description": "Undefined named entity in a double-quoted attribute value ending in semicolon and whose name starts with a known entity name.",
                "input":"<h a=\"&noti;\">",
                "output": [["StartTag", "h", {"a": "&noti;"}]]
            }"#,
        );

        assert!(test.description.starts_with("Undefined"));
        assert_eq!(test.input, "<h a=\"&noti;\">");
        assert_eq!(
            test.output,
            &[Token::StartTag {
                name: "h".into(),
                attributes: HashMap::from([("a".into(), "&noti;".into())]),
                is_self_closing: false,
            }],
        );
    }

    #[test]
    fn domjs_test_3() {
        let test = parse(
            r#"{
                "description":"CR in bogus comment state",
                "input":"<?\u000d",
                "output":[["Comment", "?\u000a"]],
                "errors":[
                    { "code": "unexpected-question-mark-instead-of-tag-name", "line": 1, "col": 2 }
                ]
            }"#,
        );

        assert_eq!(test.description, "CR in bogus comment state");
    }

    #[test]
    fn domjs_test_267() {
        let test = parse(
            r#"{
                "description":"space EOF after doctype ",
                "input":"<!DOCTYPE html ",
                "output":[["DOCTYPE", "html", null, null , false]],
                "errors":[
                    { "code": "eof-in-doctype", "line": 1, "col": 16 }
                ]
            }"#,
        );

        assert_eq!(test.description, "space EOF after doctype ");

        if let Token::DocType { name, .. } = &test.output[0] {
            assert_eq!(name, &Some("html".into()));
        } else {
            panic!();
        };

        let error = &test.errors[0];
        assert_eq!(
            error,
            &TokenError {
                code: "eof-in-doctype".into(),
                line: 1,
                col: 16
            }
        );
    }

    #[test]
    fn xml_violation_tests() {
        let input = r#"
        {"xmlViolationTests": [
            {"description":"Non-XML character",
            "input":"a\uFFFFb",
            "output":[["Character","a\uFFFDb"]]},

            {"description":"Non-XML space",
            "input":"a\u000Cb",
            "output":[["Character","a b"]]},

            {"description":"Double hyphen in comment",
            "input":"<!-- foo -- bar -->",
            "output":[["Comment"," foo - - bar "]]},

            {"description":"FF between attributes",
            "input":"<a b=''\u000Cc=''>",
            "output":[["StartTag","a",{"b":"","c":""}]]}
        ]}"#;

        let fixtures: FixtureFile = serde_json::from_str(input).expect("failed to parse");

        if let FixtureFile::XmlTests { tests } = fixtures {
            assert_eq!(tests.len(), 4);
        } else {
            panic!()
        };
    }

    #[test]
    fn test2_test_3() {
        let input = r#"
        {"description":"DOCTYPE without name",
        "input":"<!DOCTYPE>",
        "output":[["DOCTYPE", null, null, null, false]],
        "errors":[
            { "code": "missing-doctype-name", "line": 1, "col": 10 }
        ]}"#;

        let test = parse(input);

        let output = &test.output[0];
        assert!(matches!(output, Token::DocType { .. }));
    }

    #[test]
    fn double_escaped() {
        let input = r#"{
            "description":"NUL in CDATA section",
            "doubleEscaped":true,
            "initialStates":["CDATA section state"],
            "input":"\\u0000]]>",
            "output":[["Character", "\\u0000"]]
        }"#;

        let test = parse(input);
        assert!(test.double_escaped);
    }
}
