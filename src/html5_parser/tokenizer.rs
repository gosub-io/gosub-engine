use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Formatter;
use crate::html5_parser::input_stream::InputStream;
use crate::html5_parser::token_states::State;

// Constants that are not directly captured as visible chars
pub const CHAR_TAB: char = '\u{0009}';
pub const CHAR_LF: char = '\u{000A}';
pub const CHAR_FF: char = '\u{000C}';
pub const CHAR_SPACE: char = '\u{0020}';
pub const CHAR_REPLACEMENT: char = '\u{FFFD}';


// Represents an attribute in the foo=bar form
pub struct Attribute {
    value: String,
    name_span: String,
    value_span: String,
}

impl Attribute {
    pub fn new(value: String, name_span: String, value_span: String) -> Self {
        Attribute{
            value,
            name_span,
            value_span,
        }
    }
}

impl fmt::Display for Attribute {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

// Represents a start tag like '<p>', or even '<br foo="bar"/>'
pub struct StartTag {
    name: String,
    self_closing: bool,
    attributes: BTreeMap<String, Attribute>,
    name_span: String
}

impl StartTag {
    pub fn new (name: String, self_closing: bool, name_span: String) -> Self {
        StartTag{
            name,
            self_closing,
            attributes: Default::default(),
            name_span,
        }
    }
}

impl fmt::Display for StartTag {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "<{}{}>", self.name, if self.self_closing { "/" } else { "" })
    }
}

// Represents an end tag </p>
pub struct EndTag {
    name: String,
    name_span: String,
}

impl EndTag {
    pub fn new(name: String, name_span: String) -> Self {
        EndTag{
            name,
            name_span,
        }
    }
}

impl fmt::Display for EndTag {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "</{}>", self.name)
    }
}

// Represents a doctype
pub struct DocType {
    force_quirks: bool,
    name: String,
    pub_identifier: Option<String>,
    sys_identifier: Option<String>,
}

impl fmt::Display for DocType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<{} {} {} {}>",
            self.name,
            if self.force_quirks { " FORCE_QUIRKS!" } else { " NO_FORCE_QUIRKS" },
            self.pub_identifier.as_ref().unwrap_or(&String::new()),
            self.sys_identifier.as_ref().unwrap_or(&String::new()),
        )
    }
}

impl DocType {
    pub fn new(name: String, force_quirks: bool, pub_id: Option<String>, sys_id: Option<String>) -> Self {
        DocType{
            force_quirks,
            name,
            pub_identifier: pub_id,
            sys_identifier: sys_id,
        }
    }
}

// Errors produced by the tokenizer
#[derive(Debug)]
pub enum Error {
    EndOfStream,
    NullEncountered,
}

// Different tokens types that can be emitted by the tokenizer
pub(crate) enum Token {
    DocType(DocType),
    StartTag(StartTag),
    EndTag(EndTag),
    Attribute(Attribute),
    Comment(String),
    String(String),
    Error {
        error: Error,
        span: String
    },
    EOF,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::DocType(doctype) => write!(f, "doctype[{}]", doctype),
            Token::StartTag(tag) => write!(f, "starttag[{}]", tag),
            Token::EndTag(tag) => write!(f, "endtag[{}]", tag),
            Token::Attribute(s) => write!(f, "attr[{}]", s),
            Token::Comment(s) => write!(f, "comment[{}]", s),
            Token::String(s) => write!(f, "str[{}]", s),
            Token::Error { error, span} => write!(f, "err[{:?} {}]", error, span),
            Token::EOF => write!(f, "eof[]"),
        }
    }
}


// The tokenizer will read the input stream and emit tokens that can be used by the parser.
pub struct Tokenizer<'a> {
    pub stream: &'a mut InputStream,    // HTML character input stream
    pub state: State,                   // Current state of the tokenizer
    pub consumed: Vec<char>,            // Current consumed characters for current token
    pub tmp_buf: Vec<char>,             // temporary buffer
    // pub emitter: &'a mut dyn Emitter,   // Emitter trait that will emit the tokens during parsing
}

impl<'a> Tokenizer<'a> {

    pub fn new(input: &'a mut InputStream /*, emitter: &'a mut dyn Emitter*/) -> Self {
        return Tokenizer{
            stream: input,
            state: State::DataState,
            consumed: vec![],
            tmp_buf: vec![],
            // emitter,
        }
    }

    // Retrieves the next token from the input stream or Token::EOF when the end is reached
    pub(crate) fn next_token(&mut self) -> Token {
        loop {
            match self.state {
                State::DataState => {
                    let c = match self.stream.read_char() {
                        Some(c) => c,
                        None => {
                            self.parse_error("EOF");
                            return Token::EOF;
                        }
                    };

                    match c {
                        '&' => self.state = State::CharacterReferenceInDataState,
                        '<' => self.state = State::TagOpenState,
                        '\u{0000}' => {
                            self.parse_error("NUL encountered in stream");
                            return Token::Error { error: Error::NullEncountered, span: String::new() };
                        }
                        _ => return Token::String(String::from(c)),
                    }
                }
                State::CharacterReferenceInDataState => {
                    // consume character reference
                    let t = match self.consume_character_reference(None, false)
                    {
                        Some(s) => Token::String(s),
                        None => Token::String(String::from('&')),
                    };

                    self.state = State::DataState;
                    return t
                }
                State::RcDataState => {}
                State::CharacterReferenceInRcDataState => {}
                State::RawTextState => {}
                State::ScriptDataState => {}
                State::PlaintextState => {}
                State::TagOpenState => {}
                State::EndTagOpenState => {}
                State::TagNameState => {}
                State::RcDataLessThanSignState => {}
                State::RcDataEndTagOpenState => {}
                State::RcDataEndTagNameState => {}
                State::RawTextLessThanSignState => {}
                State::RawTextEndTagOpenState => {}
                State::RawTextEndTagNameState => {}
                State::ScriptDataLessThenSignState => {}
                State::ScriptDataEndTagOpenState => {}
                State::ScriptDataEndTagNameState => {}
                State::ScriptDataEscapeStartState => {}
                State::ScriptDataEscapeStartDashState => {}
                State::ScriptDataEscapedState => {}
                State::ScriptDataEscapedDashState => {}
                State::ScriptDataEscapedLessThanSignState => {}
                State::ScriptDataEscapedEndTagOpenState => {}
                State::ScriptDataEscapedEndTagNameState => {}
                State::ScriptDataDoubleEscapeStartState => {}
                State::ScriptDataDoubleEscapedState => {}
                State::ScriptDataDoubleEscapedDashState => {}
                State::ScriptDataDoubleEscapedDashDashState => {}
                State::ScriptDataDoubleEscapedLessThanSignState => {}
                State::ScriptDataDoubleEscapeEndState => {}
                State::BeforeAttributeNameState => {}
                State::AttributeNameState => {}
                State::BeforeAttributeValueState => {}
                State::AttributeValueDoubleQuotedState => {}
                State::AttributeValueSingleQuotedState => {}
                State::AttributeValueUnquotedState => {}
                State::CharacterReferenceInAttributeValueState => {}
                State::AfterAttributeValueQuotedState => {}
                State::SelfClosingStartState => {}
                State::BogusCommentState => {}
                State::MarkupDeclarationOpenState => {}
                State::CommentStartState => {}
                State::CommentStartDashState => {}
                State::CommentState => {}
                State::CommentEndDashState => {}
                State::CommentEndState => {}
                State::CommentEndBangState => {}
                State::DocTypeState => {}
                State::BeforeDocTypeNameState => {}
                State::DocTypeNameState => {}
                State::AfterDocTypeNameState => {}
                State::AfterDocTypePublicKeywordState => {}
                State::BeforeDocTypePublicIdentifierState => {}
                State::DocTypePublicIdentifierDoubleQuotedState => {}
                State::DocTypePublicIdentifierSingleQuotedState => {}
                State::AfterDoctypePublicIdentifierState => {}
                State::BetweenDocTypePublicAndSystemIdentifiersState => {}
                State::AfterDocTypeSystemKeywordState => {}
                State::BeforeDocTypeSystemIdentifiedState => {}
                State::DocTypeSystemIdentifierDoubleQuotedState => {}
                State::DocTypeSystemIdentifierSingleQuotedState => {}
                State::AfterDocTypeSystemIdentifiedState => {}
                State::BogusDocTypeState => {}
                State::CDataSectionState => {}
            }
        }

        // return Token::Error{error: Error::EndOfStream, span: String::from("")}
    }

    // Consumes the given char so it can be stored in the next output token
    pub(crate) fn consume(&mut self, c: char) {
        // Add c to the current token data
        self.consumed.push(c)
    }

    // Return the length of the current consumed array. This allows easy return to a previous
    // state if tokenizing needs to return.
    pub(crate) fn get_consume_len(&self) -> usize {
        return self.consumed.len();
    }

    // Resize the consumed array to the given len. Useful when we need to backtrack to a previous consumption state
    pub(crate) fn reset_consume_len(&mut self, len: usize)
    {
        self.consumed.resize(len, 0 as char);
    }

    pub fn get_consumed_str(&self) -> String {
        self.consumed.iter().collect()
    }

    // Clears the current consume buffer
    pub(crate) fn clear_consume_buffer(&mut self) {
        self.consumed.clear()
    }

    // Creates a parser log error message
    pub(crate) fn parse_error(&mut self, _str: &str) {
        // Add to parse log
        println!("parse_error: {}", _str)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokens() {
        let t = Token::Comment(String::from("this is a comment"));
        assert_eq!("comment[this is a comment]", t.to_string());

        let t = Token::String(String::from("this is a string"));
        assert_eq!("str[this is a string]", t.to_string());

        let t = Token::StartTag(StartTag::new(String::from("tag"), true, String::from("")));
        assert_eq!("starttag[<tag/>]", t.to_string());

        let t = Token::StartTag(StartTag::new(String::from("tag"), false, String::from("")));
        assert_eq!("starttag[<tag>]", t.to_string());

        let t = Token::EndTag(EndTag::new(String::from("tag"), String::from("")));
        assert_eq!("endtag[</tag>]", t.to_string());

        let t = Token::DocType(DocType::new(String::from("html"), true, Option::from(String::from("foo")), Option::from(String::from("bar"))));
        assert_eq!("doctype[<html  FORCE_QUIRKS! foo bar>]", t.to_string());
    }
}