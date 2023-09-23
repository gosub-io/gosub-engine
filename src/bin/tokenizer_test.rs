use std::{env, fs, io};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use serde_json::Value;
extern crate regex;
use regex::Regex;
use gosub_engine::html5_parser::error_logger::ErrorLogger;

use gosub_engine::html5_parser::input_stream::InputStream;
use gosub_engine::html5_parser::tokenizer::state::{State as TokenState};
use gosub_engine::html5_parser::tokenizer::{Options, Tokenizer};
use gosub_engine::html5_parser::tokenizer::token::{Attribute, Token, TokenTrait, TokenType};

#[macro_use]
extern crate serde_derive;

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

pub struct TestResults{
    tests: usize,               // Number of tests (as defined in the suite)
    assertions: usize,          // Number of assertions (different combinations of input/output per test)
    succeeded: usize,           // How many succeeded assertions
    failed: usize,              // How many failed assertions
    failed_position: usize,     // How many failed assertions where position is not correct
}

fn main () -> io::Result<()> {
    let default_dir = "./html5lib-tests";
    let dir = env::args().nth(1).unwrap_or(default_dir.to_string());

    let mut results = TestResults{
        tests: 0,
        assertions: 0,
        succeeded: 0,
        failed: 0,
        failed_position: 0,
    };
    
    for entry in fs::read_dir(dir + "/tokenizer")? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() || path.extension().unwrap() != "test" {
            continue;
        }

        let contents = fs::read_to_string(&path)?;
        let container = serde_json::from_str(&contents);
        if container.is_err() {
            continue;
        }
        let container: Root = container.unwrap();

        println!("🏃‍♂️ Running {} tests from 🗄️ {:?}", container.tests.len(), path);

        for test in container.tests {
            run_token_test(&test, &mut results)
        }
    }

    println!("🏁 Tests completed: Ran {} tests, {} assertions, {} succeeded, {} failed ({} position failures)", results.tests, results.assertions, results.succeeded, results.failed, results.failed_position);
    Ok(())
}

fn run_token_test(test: &Test, results: &mut TestResults)
{
    println!("🧪 Running test: {}", test.description);

    results.tests += 1;

    // If no initial state is given, assume Data state
    let mut states = test.initial_states.clone();
    if states.is_empty() {
        states.push(String::from("Data state"));
    }


    for state in states.iter() {
        let state= match state.as_str() {
            "PLAINTEXT state" => TokenState::PlaintextState,
            "RAWTEXT state" => TokenState::RawTextState,
            "RCDATA state" => TokenState::RcDataState,
            "Script data state" => TokenState::ScriptDataState,
            "CDATA section state" => TokenState::CDataSectionState,
            "Data state" => TokenState::DataState,
            _ => panic!("unknown state found in test: {} ", state)
        };

        let mut is = InputStream::new();
        let input = if test.double_escaped.unwrap_or(false) {
            escape(test.input.as_str())
        } else {
            test.input.to_string()
        };
        is.read_from_str(input.as_str(), None);

        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
        let mut tokenizer = Tokenizer::new(&mut is, Some(Options{
            initial_state: state,
            last_start_tag: test.last_start_tag.clone().unwrap_or(String::from("")),
        }), error_logger.clone());

        // If there is no output, still do an (initial) next token so the parser can generate
        // errors.
        if test.output.is_empty() {
            tokenizer.next_token();
        }

        // There can be multiple tokens to match. Make sure we match all of them
        for expected_token in test.output.iter() {
            let t = tokenizer.next_token();
            if !match_token(t, expected_token, test.double_escaped.unwrap_or(false)) {
                results.assertions += 1;
                results.failed += 1;
            }
        }

        let borrowed_error_logger = error_logger.borrow();
        if borrowed_error_logger.get_errors().len() != test.errors.len() {
            println!("❌ Unexpected errors found (wanted {}, got {}): ", test.errors.len(), borrowed_error_logger.get_errors().len());
            for want_err in &test.errors {
                println!("     * Want: '{}' at {}:{}", want_err.code, want_err.line, want_err.col);
            }
            for got_err in borrowed_error_logger.get_errors() {
                println!("     * Got: '{}' at {}:{}", got_err.message, got_err.line, got_err.col);
            }
            results.assertions += 1;
            results.failed += 1;
        }

        // Check error messages
        for error in &test.errors {
            match match_error(&tokenizer, &error) {
                ErrorResult::Failure => {
                    results.assertions += 1;
                    results.failed += 1;
                },
                ErrorResult::PositionFailure => {
                    results.assertions += 1;
                    results.failed += 1;
                    results.failed_position += 1;
                },
                ErrorResult::Success => {
                    results.assertions += 1;
                    results.succeeded += 1;
                }
            }
        }
    }

    println!("----------------------------------------");
}

#[derive(PartialEq)]
enum ErrorResult {
    Success,            // Found the correct error
    Failure,            // Didn't find the error (not even with incorrect position)
    PositionFailure,    // Found the error, but on a incorrect position
}

fn match_error(tokenizer: &Tokenizer, expected_err: &Error) -> ErrorResult {
    // Iterate all generated errors to see if we have an exact match
    for got_err in tokenizer.get_error_logger().get_errors() {
        if got_err.message == expected_err.code && got_err.line as i64 == expected_err.line && got_err.col as i64 == expected_err.col {
            // Found an exact match
            println!("✅ Found parse error '{}' at {}:{}", got_err.message, got_err.line, got_err.col);

            return ErrorResult::Success;
        }
    }

    // Try and find an error that matches the code, but has a different line/pos. Even though
    // it's not always correct, it might be a off-by-one position.
    let mut result = ErrorResult::Failure;
    for got_err in tokenizer.get_error_logger().get_errors() {
        if got_err.message == expected_err.code {
            if got_err.line as i64 != expected_err.line || got_err.col as i64 != expected_err.col {
                // println!("❌ Expected error '{}' at {}:{}", expected_err.code, expected_err.line, expected_err.col);
                result = ErrorResult::PositionFailure;
                break;
            }
        }
    }

    println!("❌ Expected error '{}' at {}:{}", expected_err.code, expected_err.line, expected_err.col);

    println!("   Parser errors generated:");
    for got_err in tokenizer.get_error_logger().get_errors() {
        println!("     * '{}' at {}:{}", got_err.message, got_err.line, got_err.col);
    }

    result
}

fn match_token(have: Token, expected: &[Value], double_escaped: bool) -> bool {
    let tp = expected.get(0).unwrap();

    let expected_token_type = match tp.as_str().unwrap() {
        "DOCTYPE" => TokenType::DocTypeToken,
        "StartTag" => TokenType::StartTagToken,
        "EndTag" => TokenType::EndTagToken,
        "Comment" => TokenType::CommentToken,
        "Character" => TokenType::TextToken,
        _ => panic!("unknown output token type {:?}", tp.as_str().unwrap())
    };

    if have.type_of() != expected_token_type {
        println!("❌ Incorrect token type found (want: {:?}, got {:?})", expected_token_type, have.type_of());
        return false;
    }

    match have {
        Token::DocTypeToken{name, force_quirks, pub_identifier, sys_identifier} => {
            if check_match_doctype(expected, name, force_quirks, pub_identifier, sys_identifier).is_err() {
                return false;
            }
        }
        Token::StartTagToken{name, attributes, is_self_closing} => {
            if check_match_starttag(expected, name, attributes, is_self_closing).is_err() {
                return false;
            }
        }
        Token::EndTagToken{name, ..} => {
            if check_match_endtag(expected, name, double_escaped).is_err() {
                return false;
            }
        }
        Token::CommentToken{value} => {
            if check_match_comment(expected, value, double_escaped).is_err() {
                return false;
            }
        }
        Token::TextToken{value} => {
            if check_match_text(expected, value, double_escaped).is_err() {
                return false;
            }
        },
        Token::EofToken => {
            println!("❌ EOF token");
            return false;
        }
    }

    println!("✅ Test passed");
    true
}

fn check_match_starttag(expected: &[Value], name: String, attributes: HashMap<String, String>, is_self_closing: bool) -> Result<(), ()> {
    let expected_name = expected.get(1).and_then(|v| v.as_str()).unwrap();
    let expected_attrs = expected.get(2).and_then(|v| v.as_object());
    let expected_self_closing = expected.get(3).and_then(|v| v.as_bool());

    if expected_name != name {
        println!("❌ Incorrect start tag (wanted: '{}', got '{}'", name, expected_name);
        return Err(());
    }

    if expected_self_closing.is_some() && expected_self_closing.unwrap() != is_self_closing {
        println!("❌ Incorrect start tag (expected selfclosing: {})", !is_self_closing);
        return Err(());
    }

    if expected_attrs.is_none() && attributes.len() == 0 {
        // No attributes to check
        return Ok(());
    }

    // Convert the expected attr to Vec<(string, string)>
    let expected_attrs: Vec<Attribute> = expected_attrs.map_or(Vec::new(), |map| {
        map.iter()
            .filter_map(|(key, value)| {
                value.as_str().map(|v| Attribute{name: key.clone(), value: v.to_string()})
            })
            .collect()
    });

    let attributes: Vec<Attribute> = attributes.iter()
        .map(|(key, value)| Attribute{name: key.clone(), value: value.clone()})
        .collect();

    let set1: HashSet<_> = expected_attrs.iter().collect();
    let set2: HashSet<_> = attributes.iter().collect();

    if set1 != set2 {
        println!("❌ Attributes mismatch");

        for attr in expected_attrs {
            println!("     * Want: '{}={}'", &attr.name, &attr.value);
        }
        for attr in attributes {
            println!("     * Got: '{}={}'", attr.name, attr.value);
        }

        return Err(())
    }

    Ok(())
}

fn check_match_comment(expected: &[Value], value: String, is_double_escaped: bool) -> Result<(), ()> {
    let output_ref = expected.get(1).unwrap().as_str().unwrap();
    let output = if is_double_escaped { escape(output_ref) } else { output_ref.to_string() };

    if value.ne(&output) {
        println!("❌ Incorrect text found in comment token");
        println!("    wanted: '{}', got: '{}'", output, value.as_str());
        return Err(());
    }

    Ok(())
}

fn check_match_text(expected: &[Value], value: String, is_double_escaped: bool) -> Result<(), ()> {
    let output_ref = expected.get(1).unwrap().as_str().unwrap();
    let output = if is_double_escaped { escape(output_ref) } else { output_ref.to_string() };

    if value.ne(&output) {
        println!("❌ Incorrect text found in text token");
        println!("    wanted: '{}', got: '{}'", output, value.as_str());
        return Err(());
    }

    Ok(())
}

fn check_match_endtag(expected: &[Value], name: String, is_double_escaped: bool) -> Result<(), ()> {
    let output_ref = expected.get(1).unwrap().as_str().unwrap();
    let output = if is_double_escaped { escape(output_ref) } else { output_ref.to_string() };

    if name.as_str() != output {
        println!("❌ Incorrect end tag");
        return Err(());
    }
    Ok(())
}

// Check if a given doctype matches the expected result
fn check_match_doctype(
    expected: &[Value],
    name: Option<String>,
    force_quirks: bool,
    pub_identifier: Option<String>,
    sys_identifier: Option<String>
) -> Result<(), ()> {
    let expected_name = expected.get(1).unwrap().as_str();
    let expected_pub = expected.get(2).unwrap().as_str();
    let expected_sys = expected.get(3).unwrap().as_str();
    let expected_quirk = expected.get(4).unwrap().as_bool();

    if expected_name.is_none() && ! name.is_none() {
        println!("❌ Incorrect doctype (no name expected, but got '{}')", name.unwrap());
        return Err(());
    }
    if expected_name.is_some() && name.is_none() {
        println!("❌ Incorrect doctype (name expected, but got none)");
        return Err(());
    }
    if expected_name.is_some() && expected_name != Some(name.clone().unwrap().as_str()) {
        println!("❌ Incorrect doctype (wanted name: '{}', got: '{}')", expected_name.unwrap(), name.unwrap().as_str());
        return Err(());
    }
    if expected_quirk.is_some() && expected_quirk.unwrap() == force_quirks {
        println!("❌ Incorrect doctype (wanted quirk: '{}')", expected_quirk.unwrap());
        return Err(());
    }
    if expected_pub != pub_identifier.as_deref() {
        println!("❌ Incorrect doctype (wanted pub id: '{:?}', got '{:?}')", expected_pub, pub_identifier);
        return Err(());
    }
    if expected_sys != sys_identifier.as_deref() {
        println!("❌ Incorrect doctype (wanted sys id: '{:?}', got '{:?}')", expected_sys, sys_identifier);
        return Err(());
    }

    Ok(())
}

fn escape(input: &str) -> String {
    let re = Regex::new(r"\\u([0-9a-fA-F]{4})").unwrap();
    re.replace_all(input, |caps: &regex::Captures| {
        let hex_val = u32::from_str_radix(&caps[1], 16).unwrap();

        // This will also convert surrogates?
        unsafe {
            char::from_u32_unchecked(hex_val).to_string()
        }
    }).into_owned()
}