pub mod state;
pub mod token;

mod character_reference;
mod replacement_tables;

use crate::html5::error_logger::{ErrorLogger, ParserError};
use crate::html5::input_stream::Element;
use crate::html5::input_stream::SeekMode::SeekCur;
use crate::html5::input_stream::{InputStream, Position};
use crate::html5::tokenizer::state::State;
use crate::html5::tokenizer::token::Token;
use crate::types::{Error, Result};
use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Constants that are not directly captured as visible chars
pub const CHAR_NUL: char = '\u{0000}';
pub const CHAR_TAB: char = '\u{0009}';
pub const CHAR_LF: char = '\u{000A}';
pub const CHAR_CR: char = '\u{000D}';
pub const CHAR_FF: char = '\u{000C}';
pub const CHAR_SPACE: char = '\u{0020}';
pub const CHAR_REPLACEMENT: char = '\u{FFFD}';

/// The tokenizer will read the input stream and emit tokens that can be used by the parser.
pub struct Tokenizer<'stream> {
    /// HTML character input stream
    pub stream: &'stream mut InputStream,
    /// Current state of the tokenizer
    pub state: State,
    /// Current consumed characters for current token
    pub consumed: String,
    /// Current attribute name that we need to store temporary in case we are parsing attributes
    pub current_attr_name: String,
    /// Current attribute value that we need to store temporary in case we are parsing attributes
    pub current_attr_value: String,
    /// Current attributes
    pub current_attrs: HashMap<String, String>,
    /// Token that is currently in the making (if any)
    pub current_token: Option<Token>,
    /// Temporary buffer
    pub temporary_buffer: String,
    /// Queue of emitted tokens. Needed because we can generate multiple tokens during iteration
    pub token_queue: Vec<Token>,
    /// The last emitted start token (or empty if none)
    pub last_start_token: String,
    /// Error logger to log errors to
    pub error_logger: Rc<RefCell<ErrorLogger>>,
}

/// Options that can be passed to the tokenizer. Mostly needed when dealing with tests.
pub struct Options {
    /// Sets the initial state of the tokenizer. Normally only needed when dealing with tests
    pub initial_state: State,
    /// Sets the last starting tag in the tokenizer. Normally only needed when dealing with tests
    pub last_start_tag: String,
}

/// Convert a character to lower case value (assumes character is in A-Z range)
macro_rules! to_lowercase {
    ($c:expr) => {
        // Converts A-Z to a-z
        ((($c) as u8) + 0x20) as char
    };
}

impl<'stream> Tokenizer<'stream> {
    /// Creates a new tokenizer with the given inputstream and additional options if any
    pub fn new(
        input: &'stream mut InputStream,
        opts: Option<Options>,
        error_logger: Rc<RefCell<ErrorLogger>>,
    ) -> Self {
        return Tokenizer {
            stream: input,
            state: opts.as_ref().map_or(State::Data, |o| o.initial_state),
            last_start_token: opts.map_or(String::new(), |o| o.last_start_tag),
            consumed: String::new(),
            current_token: None,
            token_queue: vec![],
            current_attr_name: String::new(),
            current_attr_value: String::new(),
            current_attrs: HashMap::new(),
            temporary_buffer: String::new(),
            error_logger,
        };
    }

    /// Returns the current position in the stream (with line/col number and position)
    pub(crate) fn get_position(&self) -> Position {
        self.stream.position
    }

    /// Retrieves the next token from the input stream or Token::EOF when the end is reached
    pub fn next_token(&mut self) -> Result<Token> {
        self.consume_stream()?;

        if self.token_queue.is_empty() {
            return Ok(Token::Eof);
        }

        Ok(self.token_queue.remove(0))
    }

    /// Returns the error logger
    pub fn get_error_logger(&self) -> Ref<ErrorLogger> {
        self.error_logger.borrow()
    }

    /// Consumes the input stream. Continues until the stream is completed or a token has been generated.
    fn consume_stream(&mut self) -> Result<()> {
        loop {
            // Something is already in the token buffer, so we can return it.
            if !self.token_queue.is_empty() {
                return Ok(());
            }

            match self.state {
                State::Data => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('&') => self.state = State::CharacterReferenceInData,
                        Element::Utf8('<') => self.state = State::TagOpen,
                        Element::Utf8(CHAR_NUL) => {
                            self.consume(c.utf8());
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                        }
                        Element::Eof => {
                            // EOF
                            // if self.has_consumed_data() {
                            //     self.emit_token(Token::TextToken { value: self.get_consumed_str() });
                            //     self.clear_consume_buffer();
                            // }
                            self.emit_token(Token::Eof);
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::CharacterReferenceInData => {
                    self.consume_character_reference(None, false);
                    self.state = State::Data;
                }
                State::RCDATA => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('&') => self.state = State::CharacterReferenceInRcData,
                        Element::Utf8('<') => self.state = State::RCDATALessThanSign,
                        Element::Eof => {
                            // if self.has_consumed_data() {
                            //     self.emit_token(Token::TextToken { value: self.get_consumed_str().clone() });
                            //     self.clear_consume_buffer();
                            // }
                            self.emit_token(Token::Eof);
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.consume(CHAR_REPLACEMENT);
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::CharacterReferenceInRcData => {
                    // consume character reference
                    self.consume_character_reference(None, false);
                    self.state = State::RCDATA;
                }
                State::RAWTEXT => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('<') => self.state = State::RawTextLessThanSign,
                        Element::Utf8(CHAR_NUL) => {
                            self.consume(CHAR_REPLACEMENT);
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                        }
                        Element::Eof => {
                            // EOF
                            // if self.has_consumed_data() {
                            //     self.emit_token(Token::TextToken { value: self.get_consumed_str() });
                            //     self.clear_consume_buffer();
                            // }
                            self.emit_token(Token::Eof);
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::ScriptData => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('<') => self.state = State::ScriptDataLessThenSign,
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            // if self.has_consumed_data() {
                            //     self.emit_token(Token::TextToken { value: self.get_consumed_str().clone() });
                            //     self.clear_consume_buffer();
                            // }
                            self.emit_token(Token::Eof);
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::PLAINTEXT => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            // if self.has_consumed_data() {
                            //     self.emit_token(Token::TextToken { value: self.get_consumed_str().clone() });
                            //     self.clear_consume_buffer();
                            // }
                            self.emit_token(Token::Eof);
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::TagOpen => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('!') => self.state = State::MarkupDeclarationOpen,
                        Element::Utf8('/') => self.state = State::EndTagOpen,
                        Element::Utf8(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::StartTag {
                                name: "".into(),
                                is_self_closing: false,
                                attributes: HashMap::new(),
                            });
                            self.stream.unread();
                            self.state = State::TagName;
                        }
                        Element::Utf8('?') => {
                            self.current_token = Some(Token::Comment("".into()));
                            self.parse_error(ParserError::UnexpectedQuestionMarkInsteadOfTagName);
                            self.stream.unread();
                            self.state = State::BogusComment;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofBeforeTagName);
                            self.consume('<');
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error(ParserError::InvalidFirstCharacterOfTagName);
                            self.consume('<');
                            self.stream.unread();
                            self.state = State::Data;
                        }
                    }
                }
                State::EndTagOpen => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::EndTag {
                                name: "".into(),
                                is_self_closing: false,
                            });
                            self.stream.unread();
                            self.state = State::TagName;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingEndTagName);
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofBeforeTagName);
                            self.consume('<');
                            self.consume('/');
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error(ParserError::InvalidFirstCharacterOfTagName);

                            self.current_token = Some(Token::Comment("".into()));
                            self.stream.unread();
                            self.state = State::BogusComment;
                        }
                    }
                }
                State::TagName => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => self.state = State::BeforeAttributeName,
                        Element::Utf8('/') => self.state = State::SelfClosingStart,
                        Element::Utf8('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Utf8(ch @ 'A'..='Z') => self.add_to_token_name(to_lowercase!(ch)),
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.add_to_token_name(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::Data;
                        }
                        _ => self.add_to_token_name(c.utf8()),
                    }
                }
                State::RCDATALessThanSign => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('/') => {
                            self.temporary_buffer.clear();
                            self.state = State::RCDATAEndTagOpen;
                        }
                        _ => {
                            self.consume('<');
                            self.stream.unread();
                            self.state = State::RCDATA;
                        }
                    }
                }
                State::RCDATAEndTagOpen => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::EndTag {
                                name: "".into(),
                                is_self_closing: false,
                            });
                            self.stream.unread();
                            self.state = State::RCDATAEndTagName;
                        }
                        _ => {
                            self.consume('<');
                            self.consume('/');
                            self.stream.unread();
                            self.state = State::RCDATA;
                        }
                    }
                }
                State::RCDATAEndTagName => {
                    let c = self.read_char();

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.state = State::BeforeAttributeName;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('/') => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.state = State::SelfClosingStart;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('>') => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.emit_current_token();
                                self.last_start_token = String::new();
                                self.state = State::Data;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.add_to_token_name(to_lowercase!(ch));
                            self.temporary_buffer.push(ch);
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.add_to_token_name(ch);
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.transition_to(State::RCDATA);
                    }
                }
                State::RawTextLessThanSign => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('/') => {
                            self.temporary_buffer.clear();
                            self.state = State::RawTextEndTagOpen;
                        }
                        _ => {
                            self.consume('<');
                            self.stream.unread();
                            self.state = State::RAWTEXT;
                        }
                    }
                }
                State::RawTextEndTagOpen => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::EndTag {
                                name: "".into(),
                                is_self_closing: false,
                            });
                            self.stream.unread();
                            self.state = State::RawTextEndTagName;
                        }
                        _ => {
                            self.consume('<');
                            self.consume('/');
                            self.stream.unread();
                            self.state = State::RAWTEXT;
                        }
                    }
                }
                State::RawTextEndTagName => {
                    let c = self.read_char();

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.state = State::BeforeAttributeName;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('/') => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.state = State::SelfClosingStart;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('>') => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.emit_current_token();
                                self.last_start_token = String::new();
                                self.state = State::Data;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.add_to_token_name(to_lowercase!(ch));
                            self.temporary_buffer.push(ch);
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.add_to_token_name(ch);
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.transition_to(State::RAWTEXT);
                    }
                }
                State::ScriptDataLessThenSign => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('/') => {
                            self.temporary_buffer.clear();
                            self.state = State::ScriptDataEndTagOpen;
                        }
                        Element::Utf8('!') => {
                            self.consume('<');
                            self.consume('!');
                            self.state = State::ScriptDataEscapeStart;
                        }
                        _ => {
                            self.consume('<');
                            self.stream.unread();
                            self.state = State::ScriptData;
                        }
                    }
                }
                State::ScriptDataEndTagOpen => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::EndTag {
                                name: "".into(),
                                is_self_closing: false,
                            });
                            self.stream.unread();
                            self.state = State::ScriptDataEndTagName;
                        }
                        _ => {
                            self.consume('<');
                            self.consume('/');
                            self.stream.unread();
                            self.state = State::ScriptData;
                        }
                    }
                }
                State::ScriptDataEndTagName => {
                    let c = self.read_char();

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.state = State::BeforeAttributeName;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('/') => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.state = State::SelfClosingStart;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('>') => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.emit_current_token();
                                self.last_start_token = String::new();
                                self.state = State::Data;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.add_to_token_name(to_lowercase!(ch));
                            self.temporary_buffer.push(ch);
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.add_to_token_name(ch);
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.transition_to(State::ScriptData);
                    }
                }
                State::ScriptDataEscapeStart => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataEscapeStartDash;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::ScriptData;
                        }
                    }
                }
                State::ScriptDataEscapeStartDash => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataEscapedDashDash;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::ScriptData;
                        }
                    }
                }
                State::ScriptDataEscaped => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataEscapedDash;
                        }
                        Element::Utf8('<') => {
                            self.state = State::ScriptDataEscapedLessThanSign;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::Data;
                        }
                        _ => {
                            self.consume(c.utf8());
                        }
                    }
                }
                State::ScriptDataEscapedDash => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataEscapedDashDash;
                        }
                        Element::Utf8('<') => {
                            self.state = State::ScriptDataEscapedLessThanSign;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataEscaped;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::Data;
                        }
                        _ => {
                            self.consume(c.utf8());
                            self.state = State::ScriptDataEscaped;
                        }
                    }
                }
                State::ScriptDataEscapedDashDash => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                        }
                        Element::Utf8('<') => {
                            self.state = State::ScriptDataEscapedLessThanSign;
                        }
                        Element::Utf8('>') => {
                            self.consume('>');
                            self.state = State::ScriptData;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataEscaped;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::Data;
                        }
                        _ => {
                            self.consume(c.utf8());
                            self.state = State::ScriptDataEscaped;
                        }
                    }
                }
                State::ScriptDataEscapedLessThanSign => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('/') => {
                            self.temporary_buffer.clear();
                            self.state = State::ScriptDataEscapedEndTagOpen;
                        }
                        Element::Utf8(ch) if ch.is_ascii_alphabetic() => {
                            self.temporary_buffer.clear();
                            self.consume('<');
                            self.stream.unread();
                            self.state = State::ScriptDataDoubleEscapeStart;
                        }
                        _ => {
                            // anything else
                            self.consume('<');
                            self.stream.unread();
                            self.state = State::ScriptDataEscaped;
                        }
                    }
                }
                State::ScriptDataEscapedEndTagOpen => {
                    let c = self.read_char();

                    match c {
                        Element::Utf8(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::EndTag {
                                name: "".into(),
                                is_self_closing: false,
                            });

                            self.stream.unread();
                            self.state = State::ScriptDataEscapedEndTagName;
                        }
                        _ => {
                            self.consume('<');
                            self.consume('/');
                            self.stream.unread();
                            self.state = State::ScriptDataEscaped;
                        }
                    }
                }
                State::ScriptDataEscapedEndTagName => {
                    let c = self.read_char();

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.state = State::BeforeAttributeName;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('/') => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.state = State::SelfClosingStart;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('>') => {
                            let current_end_tag_name = match &self.current_token {
                                Some(Token::EndTag { name, .. }) => name,
                                _ => "",
                            };
                            if self.is_appropriate_end_token(current_end_tag_name) {
                                self.emit_current_token();
                                self.last_start_token = String::new();
                                self.state = State::Data;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.add_to_token_name(to_lowercase!(ch));
                            self.temporary_buffer.push(ch);
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.add_to_token_name(ch);
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.transition_to(State::ScriptDataEscaped);
                    }
                }
                State::ScriptDataDoubleEscapeStart => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE)
                        | Element::Utf8('/')
                        | Element::Utf8('>') => {
                            if self.temporary_buffer == "script" {
                                self.state = State::ScriptDataDoubleEscaped;
                            } else {
                                self.state = State::ScriptDataEscaped;
                            }
                            self.consume(c.utf8());
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.temporary_buffer.push(to_lowercase!(ch));
                            self.consume(ch);
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.temporary_buffer.push(ch);
                            self.consume(ch);
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::ScriptDataEscaped;
                        }
                    }
                }
                State::ScriptDataDoubleEscaped => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataDoubleEscapedDash;
                        }
                        Element::Utf8('<') => {
                            self.consume('<');
                            self.state = State::ScriptDataDoubleEscapedLessThanSign;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::Data;
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::ScriptDataDoubleEscapedDash => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::ScriptDataDoubleEscapedDashDash;
                            self.consume('-');
                        }
                        Element::Utf8('<') => {
                            self.state = State::ScriptDataDoubleEscapedLessThanSign;
                            self.consume('<');
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::Data;
                        }
                        _ => {
                            self.consume(c.utf8());
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                    }
                }
                State::ScriptDataDoubleEscapedDashDash => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => self.consume('-'),
                        Element::Utf8('<') => {
                            self.consume('<');
                            self.state = State::ScriptDataDoubleEscapedLessThanSign;
                        }
                        Element::Utf8('>') => {
                            self.consume('>');
                            self.state = State::ScriptData;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::Data;
                        }
                        _ => {
                            self.consume(c.utf8());
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                    }
                }
                State::ScriptDataDoubleEscapedLessThanSign => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('/') => {
                            self.temporary_buffer.clear();
                            self.consume('/');
                            self.state = State::ScriptDataDoubleEscapeEnd;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                    }
                }
                State::ScriptDataDoubleEscapeEnd => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE)
                        | Element::Utf8('/')
                        | Element::Utf8('>') => {
                            if self.temporary_buffer == "script" {
                                self.state = State::ScriptDataEscaped;
                            } else {
                                self.state = State::ScriptDataDoubleEscaped;
                            }
                            self.consume(c.utf8());
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.temporary_buffer.push(to_lowercase!(ch));
                            self.consume(ch);
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.temporary_buffer.push(ch);
                            self.consume(ch);
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                    }
                }
                State::BeforeAttributeName => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // Ignore character
                        }
                        Element::Utf8('/') | Element::Utf8('>') | Element::Eof => {
                            self.stream.unread();
                            self.state = State::AfterAttributeName;
                        }
                        Element::Utf8('=') => {
                            self.parse_error(ParserError::UnexpectedEqualsSignBeforeAttributeName);

                            self.store_and_clear_current_attribute();
                            self.current_attr_name.push(c.utf8());

                            self.state = State::AttributeName;
                        }
                        _ => {
                            // Store an existing attribute if any and clear
                            self.store_and_clear_current_attribute();

                            self.stream.unread();
                            self.state = State::AttributeName;
                        }
                    }
                }
                State::AttributeName => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE)
                        | Element::Utf8('/')
                        | Element::Utf8('>')
                        | Element::Eof => {
                            if self.attr_already_exists() {
                                self.parse_error(ParserError::DuplicateAttribute);
                            }
                            self.stream.unread();

                            self.state = State::AfterAttributeName
                        }
                        Element::Utf8('=') => {
                            if self.attr_already_exists() {
                                self.parse_error(ParserError::DuplicateAttribute);
                            }
                            self.state = State::BeforeAttributeValue
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.current_attr_name.push(to_lowercase!(ch));
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.current_attr_name.push(CHAR_REPLACEMENT);
                        }
                        Element::Utf8('"') | Element::Utf8('\'') | Element::Utf8('<') => {
                            self.parse_error(ParserError::UnexpectedCharacterInAttributeName);
                            self.current_attr_name.push(c.utf8());
                        }
                        _ => self.current_attr_name.push(c.utf8()),
                    }
                }
                State::AfterAttributeName => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // Ignore
                        }
                        Element::Utf8('/') => self.state = State::SelfClosingStart,
                        Element::Utf8('=') => self.state = State::BeforeAttributeValue,
                        Element::Utf8('>') => {
                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::Data;
                        }
                        _ => {
                            self.store_and_clear_current_attribute();
                            self.stream.unread();
                            self.state = State::AttributeName;
                        }
                    }
                }
                State::BeforeAttributeValue => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // Ignore
                        }
                        Element::Utf8('"') => self.state = State::AttributeValueDoubleQuoted,
                        Element::Utf8('\'') => {
                            self.state = State::AttributeValueSingleQuoted;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingAttributeValue);

                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::AttributeValueUnquoted;
                        }
                    }
                }
                State::AttributeValueDoubleQuoted => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('"') => self.state = State::AfterAttributeValueQuoted,
                        Element::Utf8('&') => {
                            self.consume_character_reference(Some(Element::Utf8('"')), true);
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.current_attr_value.push(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::Data;
                        }
                        _ => {
                            self.current_attr_value.push(c.utf8());
                        }
                    }
                }
                State::AttributeValueSingleQuoted => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('\'') => self.state = State::AfterAttributeValueQuoted,
                        Element::Utf8('&') => {
                            self.consume_character_reference(Some(Element::Utf8('\'')), true);
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.current_attr_value.push(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::Data;
                        }
                        _ => {
                            self.current_attr_value.push(c.utf8());
                        }
                    }
                }
                State::AttributeValueUnquoted => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            self.state = State::BeforeAttributeName;
                        }
                        Element::Utf8('&') => {
                            self.consume_character_reference(Some(Element::Utf8('>')), true);
                        }
                        Element::Utf8('>') => {
                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.current_attr_value.push(CHAR_REPLACEMENT);
                        }
                        Element::Utf8('"')
                        | Element::Utf8('\'')
                        | Element::Utf8('<')
                        | Element::Utf8('=')
                        | Element::Utf8('`') => {
                            self.parse_error(
                                ParserError::UnexpectedCharacterInUnquotedAttributeValue,
                            );
                            self.current_attr_value.push(c.utf8());
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::Data;
                        }
                        _ => {
                            self.current_attr_value.push(c.utf8());
                        }
                    }
                }
                // State::CharacterReferenceInAttributeValueState => {}
                State::AfterAttributeValueQuoted => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => self.state = State::BeforeAttributeName,
                        Element::Utf8('/') => self.state = State::SelfClosingStart,
                        Element::Utf8('>') => {
                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error(ParserError::MissingWhitespaceBetweenAttributes);
                            self.stream.unread();
                            self.state = State::BeforeAttributeName;
                        }
                    }
                }
                State::SelfClosingStart => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('>') => {
                            self.set_is_closing_in_current_token(true);
                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error(ParserError::UnexpectedSolidusInTag);
                            self.stream.unread();
                            self.state = State::BeforeAttributeName;
                        }
                    }
                }
                State::BogusComment => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.add_to_token_value(CHAR_REPLACEMENT);
                        }
                        _ => {
                            self.add_to_token_value(c.utf8());
                        }
                    }
                }
                State::MarkupDeclarationOpen => {
                    if self.stream.look_ahead_slice(2) == "--" {
                        self.current_token = Some(Token::Comment("".into()));

                        // Skip the two -- signs
                        self.stream.seek(SeekCur, 2);

                        self.state = State::CommentStart;
                        continue;
                    }

                    if self.stream.look_ahead_slice(7).to_uppercase() == "DOCTYPE" {
                        self.stream.seek(SeekCur, 7);
                        self.state = State::DocType;
                        continue;
                    }

                    if self.stream.look_ahead_slice(7) == "[CDATA[" {
                        self.stream.seek(SeekCur, 7);

                        // @TODO: If there is an adjusted current node and it is not an element in the HTML namespace,
                        // then switch to the CDATA section state. Otherwise, this is a cdata-in-html-content parse error.
                        // Create a comment token whose data is the "[CDATA[" string. Switch to the bogus comment state.
                        self.parse_error(ParserError::CdataInHtmlContent);
                        self.current_token = Some(Token::Comment("[CDATA[".into()));

                        self.state = State::BogusComment;
                        continue;
                    }

                    self.stream.seek(SeekCur, 1);
                    self.parse_error(ParserError::IncorrectlyOpenedComment);
                    self.stream.unread();
                    self.current_token = Some(Token::Comment("".into()));

                    self.state = State::BogusComment;
                }
                State::CommentStart => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::CommentStartDash;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptClosingOfEmptyComment);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::Comment;
                        }
                    }
                }
                State::CommentStartDash => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::CommentEnd;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptClosingOfEmptyComment);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInComment);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.add_to_token_value('-');
                            self.stream.unread();
                            self.state = State::Comment;
                        }
                    }
                }
                State::Comment => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('<') => {
                            self.add_to_token_value(c.utf8());
                            self.state = State::CommentLessThanSign;
                        }
                        Element::Utf8('-') => self.state = State::CommentEndDash,
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.add_to_token_value(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInComment);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.add_to_token_value(c.utf8());
                        }
                    }
                }
                State::CommentLessThanSign => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('!') => {
                            self.add_to_token_value(c.utf8());
                            self.state = State::CommentLessThanSignBang;
                        }
                        Element::Utf8('<') => {
                            self.add_to_token_value(c.utf8());
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::Comment;
                        }
                    }
                }
                State::CommentLessThanSignBang => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::CommentLessThanSignBangDash;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::Comment;
                        }
                    }
                }
                State::CommentLessThanSignBangDash => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::CommentLessThanSignBangDashDash;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::CommentEndDash;
                        }
                    }
                }
                State::CommentLessThanSignBangDashDash => {
                    let c = self.read_char();
                    match c {
                        Element::Eof | Element::Utf8('>') => {
                            self.stream.unread();
                            self.state = State::CommentEnd;
                        }
                        _ => {
                            self.parse_error(ParserError::NestedComment);
                            self.stream.unread();
                            self.state = State::CommentEnd;
                        }
                    }
                }
                State::CommentEndDash => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::CommentEnd;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInComment);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.add_to_token_value('-');
                            self.stream.unread();
                            self.state = State::Comment;
                        }
                    }
                }
                State::CommentEnd => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Utf8('!') => self.state = State::CommentEndBang,
                        Element::Utf8('-') => self.add_to_token_value('-'),
                        Element::Eof => {
                            self.parse_error(ParserError::EofInComment);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.add_to_token_value('-');
                            self.add_to_token_value('-');
                            self.stream.unread();
                            self.state = State::Comment;
                        }
                    }
                }
                State::CommentEndBang => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('-') => {
                            self.add_to_token_value('-');
                            self.add_to_token_value('-');
                            self.add_to_token_value('!');

                            self.state = State::CommentEndDash;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::IncorrectlyClosedComment);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInComment);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.add_to_token_value('-');
                            self.add_to_token_value('-');
                            self.add_to_token_value('!');
                            self.stream.unread();
                            self.state = State::Comment;
                        }
                    }
                }
                State::DocType => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => self.state = State::BeforeDocTypeName,
                        Element::Utf8('>') => {
                            self.stream.unread();
                            self.state = State::BeforeDocTypeName;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);

                            self.emit_token(Token::DocType {
                                name: None,
                                force_quirks: true,
                                pub_identifier: None,
                                sys_identifier: None,
                            });

                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error(ParserError::MissingWhitespaceBeforeDoctypeName);
                            self.stream.unread();
                            self.state = State::BeforeDocTypeName;
                        }
                    }
                }
                State::BeforeDocTypeName => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.current_token = Some(Token::DocType {
                                name: None,
                                force_quirks: false,
                                pub_identifier: None,
                                sys_identifier: None,
                            });

                            self.add_to_token_name(to_lowercase!(ch));
                            self.state = State::DocTypeName;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.current_token = Some(Token::DocType {
                                name: None,
                                force_quirks: false,
                                pub_identifier: None,
                                sys_identifier: None,
                            });

                            self.add_to_token_name(CHAR_REPLACEMENT);
                            self.state = State::DocTypeName;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingDoctypeName);
                            self.emit_token(Token::DocType {
                                name: None,
                                force_quirks: true,
                                pub_identifier: None,
                                sys_identifier: None,
                            });

                            self.state = State::Data;
                        }

                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);

                            self.emit_token(Token::DocType {
                                name: None,
                                force_quirks: true,
                                pub_identifier: None,
                                sys_identifier: None,
                            });

                            self.state = State::Data;
                        }
                        _ => {
                            self.current_token = Some(Token::DocType {
                                name: None,
                                force_quirks: false,
                                pub_identifier: None,
                                sys_identifier: None,
                            });

                            self.add_to_token_name(c.utf8());
                            self.state = State::DocTypeName;
                        }
                    }
                }
                State::DocTypeName => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => self.state = State::AfterDocTypeName,
                        Element::Utf8('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Utf8(ch @ 'A'..='Z') => self.add_to_token_name(to_lowercase!(ch)),
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.add_to_token_name(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => self.add_to_token_name(c.utf8()),
                    }
                }
                State::AfterDocTypeName => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.stream.unread();
                            if self.stream.look_ahead_slice(6).to_uppercase() == "PUBLIC" {
                                self.stream.seek(SeekCur, 6);
                                self.state = State::AfterDocTypePublicKeyword;
                                continue;
                            }
                            if self.stream.look_ahead_slice(6).to_uppercase() == "SYSTEM" {
                                self.stream.seek(SeekCur, 6);
                                self.state = State::AfterDocTypeSystemKeyword;
                                continue;
                            }
                            // Make sure the parser is on the correct position again since we just
                            // unread the character
                            self.stream.seek(SeekCur, 1);
                            self.parse_error(ParserError::InvalidCharacterSequenceAfterDoctypeName);
                            self.stream.seek(SeekCur, -1);
                            self.set_quirks_mode(true);
                            self.stream.unread();
                            self.state = State::BogusDocType;
                        }
                    }
                }
                State::AfterDocTypePublicKeyword => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            self.state = State::BeforeDocTypePublicIdentifier
                        }
                        Element::Utf8('"') => {
                            self.parse_error(
                                ParserError::MissingWhitespaceAfterDoctypePublicKeyword,
                            );
                            self.set_public_identifier(String::new());
                            self.state = State::DocTypePublicIdentifierDoubleQuoted;
                        }
                        Element::Utf8('\'') => {
                            self.parse_error(
                                ParserError::MissingWhitespaceAfterDoctypePublicKeyword,
                            );
                            self.set_public_identifier(String::new());
                            self.state = State::DocTypePublicIdentifierSingleQuoted;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingDoctypePublicIdentifier);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypePublicIdentifier,
                            );
                            self.stream.unread();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocType;
                        }
                    }
                }
                State::BeforeDocTypePublicIdentifier => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8('"') => {
                            self.set_public_identifier(String::new());
                            self.state = State::DocTypePublicIdentifierDoubleQuoted;
                        }
                        Element::Utf8('\'') => {
                            self.set_public_identifier(String::new());
                            self.state = State::DocTypePublicIdentifierSingleQuoted;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingDoctypePublicIdentifier);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.stream.unread();
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypePublicIdentifier,
                            );
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocType;
                        }
                    }
                }
                State::DocTypePublicIdentifierDoubleQuoted => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('"') => self.state = State::AfterDoctypePublicIdentifier,
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.add_public_identifier(CHAR_REPLACEMENT);
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptDoctypePublicIdentifier);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => self.add_public_identifier(c.utf8()),
                    }
                }
                State::DocTypePublicIdentifierSingleQuoted => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('\'') => self.state = State::AfterDoctypePublicIdentifier,
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.add_public_identifier(CHAR_REPLACEMENT);
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptDoctypePublicIdentifier);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => self.add_public_identifier(c.utf8()),
                    }
                }
                State::AfterDoctypePublicIdentifier => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            self.state = State::BetweenDocTypePublicAndSystemIdentifiers
                        }
                        Element::Utf8('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Utf8('"') => {
                            self.parse_error(ParserError::MissingWhitespaceBetweenDoctypePublicAndSystemIdentifiers);
                            self.set_system_identifier(String::new());
                            self.state = State::DocTypeSystemIdentifierDoubleQuoted;
                        }
                        Element::Utf8('\'') => {
                            self.parse_error(ParserError::MissingWhitespaceBetweenDoctypePublicAndSystemIdentifiers);
                            self.set_system_identifier(String::new());
                            self.state = State::DocTypeSystemIdentifierSingleQuoted;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypeSystemIdentifier,
                            );
                            self.stream.unread();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocType;
                        }
                    }
                }
                State::BetweenDocTypePublicAndSystemIdentifiers => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Utf8('"') => {
                            self.set_system_identifier(String::new());
                            self.state = State::DocTypeSystemIdentifierDoubleQuoted;
                        }
                        Element::Utf8('\'') => {
                            self.set_system_identifier(String::new());
                            self.state = State::DocTypeSystemIdentifierSingleQuoted;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypeSystemIdentifier,
                            );
                            self.stream.unread();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocType;
                        }
                    }
                }
                State::AfterDocTypeSystemKeyword => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            self.state = State::BeforeDocTypeSystemIdentifier
                        }
                        Element::Utf8('"') => {
                            self.parse_error(
                                ParserError::MissingWhitespaceAfterDoctypeSystemKeyword,
                            );
                            self.set_system_identifier(String::new());
                            self.state = State::DocTypeSystemIdentifierDoubleQuoted;
                        }
                        Element::Utf8('\'') => {
                            self.parse_error(
                                ParserError::MissingWhitespaceAfterDoctypeSystemKeyword,
                            );
                            self.set_system_identifier(String::new());
                            self.state = State::DocTypeSystemIdentifierSingleQuoted;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingDoctypeSystemIdentifier);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypeSystemIdentifier,
                            );
                            self.stream.unread();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocType;
                        }
                    }
                }
                State::BeforeDocTypeSystemIdentifier => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8('"') => {
                            self.set_system_identifier(String::new());
                            self.state = State::DocTypeSystemIdentifierDoubleQuoted;
                        }
                        Element::Utf8('\'') => {
                            self.set_system_identifier(String::new());
                            self.state = State::DocTypeSystemIdentifierSingleQuoted;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingDoctypeSystemIdentifier);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypeSystemIdentifier,
                            );
                            self.stream.unread();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocType;
                        }
                    }
                }
                State::DocTypeSystemIdentifierDoubleQuoted => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('"') => self.state = State::AfterDocTypeSystemIdentifier,
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.add_system_identifier(CHAR_REPLACEMENT);
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptDoctypeSystemIdentifier);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => self.add_system_identifier(c.utf8()),
                    }
                }
                State::DocTypeSystemIdentifierSingleQuoted => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('\'') => self.state = State::AfterDocTypeSystemIdentifier,
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.add_system_identifier(CHAR_REPLACEMENT);
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptDoctypeSystemIdentifier);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => self.add_system_identifier(c.utf8()),
                    }
                }
                State::AfterDocTypeSystemIdentifier => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::UnexpectedCharacterAfterDoctypeSystemIdentifier,
                            );
                            self.stream.unread();
                            self.state = State::BogusDocType;
                        }
                    }
                }
                State::BogusDocType => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter)
                        }
                        Element::Eof => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            // ignore
                        }
                    }
                }
                State::CDATASection => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(']') => {
                            self.state = State::CDATASectionBracket;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInCdata);
                            self.state = State::Data;
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::CDATASectionBracket => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(']') => self.state = State::CDATASectionEnd,
                        _ => {
                            self.consume(']');
                            self.stream.unread();
                            self.state = State::CDATASection;
                        }
                    }
                }
                State::CDATASectionEnd => {
                    let c = self.read_char();
                    match c {
                        Element::Utf8(']') => self.consume(']'),
                        Element::Utf8('>') => self.state = State::Data,
                        _ => {
                            self.consume(']');
                            self.consume(']');
                            self.stream.unread();
                            self.state = State::CDATASection;
                        }
                    }
                }
                _ => {
                    panic!("state {:?} not implemented", self.state);
                }
            }
        }
    }

    /// This macro reads a character from the input stream and optionally generates (tokenization)
    /// errors if the character is not valid.
    fn read_char(&mut self) -> Element {
        let mut c = self.stream.read_char();
        match c {
            Element::Surrogate(..) => {
                self.parse_error(ParserError::SurrogateInInputStream);
                c = Element::Utf8(CHAR_REPLACEMENT);
            }
            Element::Utf8(c) if self.is_control_char(c as u32) => {
                self.parse_error(ParserError::ControlCharacterInInputStream);
            }
            Element::Utf8(c) if self.is_noncharacter(c as u32) => {
                self.parse_error(ParserError::NoncharacterInInputStream);
            }
            _ => {}
        }

        c
    }

    /// Adds the given character to the current token's value (if applicable)
    fn add_to_token_value(&mut self, c: char) {
        if let Some(Token::Comment(value)) = &mut self.current_token {
            value.push(c);
        }
    }

    /// Sets the public identifier of the current token (if applicable)
    fn set_public_identifier(&mut self, s: String) {
        if let Some(Token::DocType { pub_identifier, .. }) = &mut self.current_token {
            *pub_identifier = Some(s);
        }
    }

    /// Adds the given character to the current token's public identifier (if applicable)
    fn add_public_identifier(&mut self, c: char) {
        if let Some(Token::DocType {
            pub_identifier: Some(pid),
            ..
        }) = &mut self.current_token
        {
            pid.push(c);
        }
    }

    /// Sets the system identifier of the current token (if applicable)
    fn set_system_identifier(&mut self, s: String) {
        if let Some(Token::DocType { sys_identifier, .. }) = &mut self.current_token {
            *sys_identifier = Some(s);
        }
    }

    /// Adds the given character to the current token's system identifier (if applicable)
    fn add_system_identifier(&mut self, c: char) {
        if let Some(Token::DocType {
            sys_identifier: Some(sid),
            ..
        }) = &mut self.current_token
        {
            sid.push(c);
        }
    }

    /// Adds the given character to the current token's name (if applicable)
    fn add_to_token_name(&mut self, c: char) {
        match &mut self.current_token {
            Some(Token::StartTag { name, .. }) => {
                name.push(c);
            }
            Some(Token::EndTag { name, .. }) => {
                name.push(c);
            }
            Some(Token::DocType { name, .. }) => {
                // Doctype can have an optional name
                match name {
                    Some(ref mut string) => string.push(c),
                    None => *name = Some(c.to_string()),
                }
            }
            _ => {}
        }
    }

    /// Emits the current stored token
    fn emit_current_token(&mut self) {
        if let Some(t) = self.current_token.take() {
            self.emit_token(t);
        }
    }

    /// Emits the given stored token. It does not have to be stored first.
    fn emit_token(&mut self, token: Token) {
        // Save the start token name if we are pushing it. This helps us in detecting matching tags.
        if let Token::StartTag { name, .. } = &token {
            self.last_start_token = String::from(name);
        }

        // If there is any consumed data, emit this first as a text token
        if self.has_consumed_data() {
            let value = self.get_consumed_str().to_string();

            self.token_queue.push(Token::Text(value.to_string()));

            self.clear_consume_buffer();
        }

        self.token_queue.push(token);
    }

    // Consumes the given character
    pub(crate) fn consume(&mut self, c: char) {
        // Add c to the current token data
        self.consumed.push(c)
    }

    /// Pushes a end-tag and changes to the given state
    fn transition_to(&mut self, state: State) {
        self.consumed.push_str("</");
        self.consumed.push_str(&self.temporary_buffer);
        self.temporary_buffer.clear();
        self.stream.unread();
        self.state = state;
    }

    /// Consumes the given string
    pub(crate) fn consume_str(&mut self, s: &str) {
        // Add s to the current token data
        self.consumed.push_str(s);
    }

    /// Return true when the given end_token matches the stored start token (ie: 'table' matches when
    /// last_start_token = 'table')
    fn is_appropriate_end_token(&self, end_token: &str) -> bool {
        self.last_start_token == end_token
    }

    /// Return the consumed string as a String
    pub fn get_consumed_str(&self) -> &str {
        &self.consumed
    }

    /// Returns true if there is anything in the consume buffer
    pub fn has_consumed_data(&self) -> bool {
        !self.consumed.is_empty()
    }

    /// Clears the current consume buffer
    pub(crate) fn clear_consume_buffer(&mut self) {
        self.consumed.clear()
    }

    /// Creates a parser log error message
    pub(crate) fn parse_error(&mut self, message: ParserError) {
        // The previous position is where the error occurred
        let pos = self.stream.get_previous_position();

        self.error_logger
            .borrow_mut()
            .add_error(pos, message.as_str());
    }

    /// Set is_closing_tag in current token
    fn set_is_closing_in_current_token(&mut self, is_closing: bool) {
        match &mut self.current_token.as_mut().unwrap() {
            Token::EndTag { .. } => {
                self.parse_error(ParserError::EndTagWithTrailingSolidus);
            }
            Token::StartTag {
                is_self_closing, ..
            } => {
                *is_self_closing = is_closing;
            }
            _ => {}
        }
    }

    /// Set force_quirk mode in current token
    fn set_quirks_mode(&mut self, quirky: bool) {
        if let Token::DocType { force_quirks, .. } = &mut self.current_token.as_mut().unwrap() {
            *force_quirks = quirky;
        }
    }

    /// Adds a new attribute to the current token
    fn set_add_attribute_to_current_token(&mut self, name: &str, value: &str) {
        if let Token::StartTag { attributes, .. } = &mut self.current_token.as_mut().unwrap() {
            attributes.insert(name.into(), value.into());
        }

        self.current_attr_name.clear()
    }

    /// Sets the given name into the current token
    fn set_name_in_current_token(&mut self, new_name: String) -> Result<()> {
        match &mut self.current_token.as_mut().expect("current token") {
            Token::StartTag { name, .. } => {
                *name = new_name;
            }
            Token::EndTag { name, .. } => {
                *name = new_name;
            }
            _ => {
                return Err(Error::Parse(
                    "trying to set the name of a non start/end tag token".into(),
                ))
            }
        }

        Ok(())
    }

    /// This function checks to see if there is already an attribute name like the one in current_attr_name.
    fn attr_already_exists(&mut self) -> bool {
        self.current_attrs.contains_key(&self.current_attr_name)
    }

    /// Saves the current attribute name and value onto the current_attrs stack, if there is anything to store
    fn store_and_clear_current_attribute(&mut self) {
        if !self.current_attr_name.is_empty()
            && !self.current_attrs.contains_key(&self.current_attr_name)
        {
            self.current_attrs.insert(
                self.current_attr_name.clone(),
                self.current_attr_value.clone(),
            );
        }

        self.current_attr_name = String::new();
        self.current_attr_value = String::new();
    }

    /// This method will add current generated attributes to the current (start) token if needed.
    fn add_stored_attributes_to_current_token(&mut self) {
        if self.current_token.is_none() {
            return;
        }
        if self.current_attrs.is_empty() {
            return;
        }

        match self.current_token.as_mut().expect("current token") {
            Token::EndTag { .. } => {
                self.parse_error(ParserError::EndTagWithAttributes);
            }
            Token::StartTag { attributes, .. } => {
                for (key, value) in &self.current_attrs {
                    attributes.insert(key.clone(), value.clone());
                }
                self.current_attrs = HashMap::new();
            }
            _ => {}
        }
    }
}
