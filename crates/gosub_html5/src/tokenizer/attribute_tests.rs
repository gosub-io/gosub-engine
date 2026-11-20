//! Attribute state that must not outlive the tag it was parsed for (GOS-413).

use crate::parser::errors::ErrorLogger;
use crate::tokenizer::state::State;
use crate::tokenizer::token::Token;
use crate::tokenizer::{Options, ParserData, Tokenizer};
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use std::cell::RefCell;
use std::rc::Rc;

/// Reads tokens up to and including the first EOF token.
fn read_until_eof(tokenizer: &mut Tokenizer) -> Vec<String> {
    let mut tokens = Vec::new();
    loop {
        let token = tokenizer.next_token(ParserData::default()).unwrap();
        let eof = matches!(token, Token::Eof { .. });
        tokens.push(token.to_string());
        if eof {
            return tokens;
        }
    }
}

fn error_codes(logger: &Rc<RefCell<ErrorLogger>>) -> Vec<String> {
    logger.borrow().get_errors().into_iter().map(|e| e.message).collect()
}

/// Tokenizes `input` from the given state and returns the tokens and the parse error codes.
fn tokenize_in(input: &str, state: State, last_start_tag: &str) -> (Vec<String>, Vec<String>) {
    let mut stream = ByteStream::from_str(input, Encoding::UTF8);
    let logger = Rc::new(RefCell::new(ErrorLogger::new()));
    let opts = Options {
        initial_state: state,
        last_start_tag: last_start_tag.to_string(),
    };
    let mut tokenizer = Tokenizer::new(&mut stream, Some(opts), logger.clone(), Location::default());
    let tokens = read_until_eof(&mut tokenizer);
    (tokens, error_codes(&logger))
}

fn tokenize(input: &str) -> (Vec<String>, Vec<String>) {
    tokenize_in(input, State::Data, "")
}

/// Tokenizes `first` up to EOF on a stream that is still open, then appends `rest` and
/// tokenizes that too. Returns the tokens of the second part and the attribute state that
/// was left behind after the first EOF.
fn tokenize_after_eof(first: &str, rest: &str) -> (Vec<String>, String, String, usize) {
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(first, Some(Encoding::UTF8));
    let logger = Rc::new(RefCell::new(ErrorLogger::new()));
    let mut tokenizer = Tokenizer::new(&mut stream, None, logger.clone(), Location::default());

    read_until_eof(&mut tokenizer);
    assert!(error_codes(&logger).iter().any(|e| e == "eof-in-tag"), "{first:?}");
    let name = tokenizer.current_attr_name.clone();
    let value = tokenizer.current_attr_value.clone();
    let stored = tokenizer.current_attrs.len();

    tokenizer.stream.append_str(rest);
    tokenizer.stream.close();
    (read_until_eof(&mut tokenizer), name, value, stored)
}

#[test]
fn end_tag_attributes_are_not_moved_to_next_start_tag() {
    let cases = [
        ("</p a=1><div b=2>", r#"<div b="2">"#),
        ("</p a=1><div a=2>", r#"<div a="2">"#),
        ("</p a=1><div>", "<div>"),
        ("</p a=1><div a>", r#"<div a="">"#),
        ("</p a='1'><div b=2>", r#"<div b="2">"#),
        (r#"</p a="1"><div b=2>"#, r#"<div b="2">"#),
        ("</p a><div b=2>", r#"<div b="2">"#),
        ("</p a=><div b=2>", r#"<div b="2">"#),
        ("</p a=1 b=2 c=3><div d=4>", r#"<div d="4">"#),
        ("</p a=1></p b=2><div c=3>", r#"<div c="3">"#),
        ("</p a=1>text<div b=2>", r#"<div b="2">"#),
        ("</p a=1><div b=2><span c=3>", r#"<span c="3">"#),
    ];

    for (input, expected) in cases {
        let (tokens, errors) = tokenize(input);
        assert!(tokens.iter().any(|t| t == "</p>"), "{input:?}: {tokens:?}");
        assert_eq!(tokens[tokens.len() - 2], expected, "{input:?}: {tokens:?}");
        assert!(
            errors.iter().any(|e| e == "end-tag-with-attributes"),
            "{input:?}: {errors:?}"
        );
        assert!(
            !errors.iter().any(|e| e == "duplicate-attribute"),
            "{input:?}: {errors:?}"
        );
    }
}

#[test]
fn end_tag_with_duplicate_attributes() {
    let (tokens, errors) = tokenize("</p a=1 a=2><div a=3>");
    assert_eq!(tokens, ["</p>", r#"<div a="3">"#, "EOF"]);
    assert_eq!(errors, ["duplicate-attribute", "end-tag-with-attributes"]);
}

#[test]
fn self_closing_end_tag_with_attributes() {
    let (tokens, errors) = tokenize(r#"</p a="1"/><div b=2>"#);
    assert_eq!(tokens, ["</p>", r#"<div b="2">"#, "EOF"]);
    assert!(
        errors.iter().any(|e| e == "end-tag-with-trailing-solidus"),
        "{errors:?}"
    );
    assert!(errors.iter().any(|e| e == "end-tag-with-attributes"), "{errors:?}");

    let (tokens, _) = tokenize("</p a/><div a>");
    assert_eq!(tokens, ["</p>", r#"<div a="">"#, "EOF"]);
}

#[test]
fn end_tag_with_attributes_at_end_of_input() {
    for input in ["</p a=1>", "<div></div a=1>"] {
        let mut stream = ByteStream::from_str(input, Encoding::UTF8);
        let logger = Rc::new(RefCell::new(ErrorLogger::new()));
        let mut tokenizer = Tokenizer::new(&mut stream, None, logger.clone(), Location::default());

        let tokens = read_until_eof(&mut tokenizer);
        assert_eq!(tokens.last().map(String::as_str), Some("EOF"), "{input:?}");
        assert_eq!(error_codes(&logger), ["end-tag-with-attributes"], "{input:?}");
        assert!(tokenizer.current_attrs.is_empty(), "{input:?}");
    }
}

#[test]
fn appropriate_end_tag_with_attributes_in_text_states() {
    let cases = [
        (State::RCDATA, "title"),
        (State::RAWTEXT, "style"),
        (State::ScriptData, "script"),
    ];

    for (state, tag) in cases {
        let input = format!("x</{tag} a=1><div b=2>");
        let (tokens, errors) = tokenize_in(&input, state, tag);
        assert_eq!(
            tokens,
            ["x", &format!("</{tag}>"), r#"<div b="2">"#, "EOF"],
            "{state:?}"
        );
        assert_eq!(errors, ["end-tag-with-attributes"], "{state:?}");

        let input = format!("x</{tag} a=1><div a=2>");
        let (tokens, _) = tokenize_in(&input, state, tag);
        assert_eq!(
            tokens,
            ["x", &format!("</{tag}>"), r#"<div a="2">"#, "EOF"],
            "{state:?}"
        );
    }
}

#[test]
fn eof_in_tag_drops_attribute_state() {
    let cases = [
        // AttributeName
        "<div a",
        // AfterAttributeName
        "<div a ",
        // BeforeAttributeValue
        "<div a=",
        // AttributeValueDoubleQuoted
        r#"<div a="1"#,
        // AttributeValueSingleQuoted
        "<div a='1",
        // AttributeValueUnquoted
        "<div a=1",
        // AfterAttributeValueQuoted
        r#"<div a="1""#,
        // SelfClosingStart
        r#"<div a="1" /"#,
        // A stored attribute plus one being read
        "<div x=0 a=1",
        // End tags
        "</div a=1",
        r#"</div x=0 a="1""#,
    ];

    for first in cases {
        let (tokens, name, value, stored) = tokenize_after_eof(first, "<p a=2 b=3>");
        assert_eq!((name.as_str(), value.as_str(), stored), ("", "", 0), "{first:?}");
        assert_eq!(tokens, [r#"<p a="2" b="3">"#, "EOF"], "{first:?}");
    }
}
