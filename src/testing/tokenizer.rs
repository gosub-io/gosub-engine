use regex::Regex;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use std::{cell::RefCell, rc::Rc};
use std::{
    fs,
    path::{Path, PathBuf},
};

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
                escape(self.input.as_str())
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
}

pub fn escape(input: &str) -> String {
    let re = Regex::new(r"\\u([0-9a-fA-F]{4})").unwrap();
    re.replace_all(input, |caps: &regex::Captures| {
        let hex_val = u32::from_str_radix(&caps[1], 16).unwrap();

        // This will also convert surrogates?
        unsafe { char::from_u32_unchecked(hex_val).to_string() }
    })
    .into_owned()
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
