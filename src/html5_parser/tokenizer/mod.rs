pub mod state;
pub mod token;

mod character_reference;
mod replacement_tables;

use crate::html5_parser::error_logger::{ErrorLogger, ParserError};
use crate::html5_parser::input_stream::Element;
use crate::html5_parser::input_stream::SeekMode::SeekCur;
use crate::html5_parser::input_stream::{InputStream, Position};
use crate::html5_parser::tokenizer::state::State;
use crate::html5_parser::tokenizer::token::Token;
use crate::types::{Error, Result};
use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

// Constants that are not directly captured as visible chars
pub const CHAR_NUL: char = '\u{0000}';
pub const CHAR_TAB: char = '\u{0009}';
pub const CHAR_LF: char = '\u{000A}';
pub const CHAR_CR: char = '\u{000D}';
pub const CHAR_FF: char = '\u{000C}';
pub const CHAR_SPACE: char = '\u{0020}';
pub const CHAR_REPLACEMENT: char = '\u{FFFD}';

// The tokenizer will read the input stream and emit tokens that can be used by the parser.
pub struct Tokenizer<'a> {
    pub stream: &'a mut InputStream, // HTML character input stream
    pub state: State,                // Current state of the tokenizer
    pub consumed: String,            // Current consumed characters for current token
    pub current_attr_name: String, // Current attribute name that we need to store temporary in case we are parsing attributes
    pub current_attr_value: String, // Current attribute value that we need to store temporary in case we are parsing attributes
    pub current_attrs: HashMap<String, String>, // Current attributes
    pub current_token: Option<Token>, // Token that is currently in the making (if any)
    pub temporary_buffer: String,   // Temporary buffer
    pub token_queue: Vec<Token>, // Queue of emitted tokens. Needed because we can generate multiple tokens during iteration
    pub last_start_token: String, // The last emitted start token (or empty if none)
    pub error_logger: Rc<RefCell<ErrorLogger>>, // Parse errors
}

pub struct Options {
    pub initial_state: State, // Sets the initial state of the tokenizer. Normally only needed when dealing with tests
    pub last_start_tag: String, // Sets the last starting tag in the tokenizer. Normally only needed when dealing with tests
}

#[macro_export]
macro_rules! read_char {
    ($self:expr) => {{
        let mut c = $self.stream.read_char();
        match c {
            Element::Surrogate(..) => {
                $self.parse_error(ParserError::SurrogateInInputStream);
                c = Element::Utf8(CHAR_REPLACEMENT);
            }
            Element::Utf8(c) if $self.is_control_char(c as u32) => {
                $self.parse_error(ParserError::ControlCharacterInInputStream);
            }
            Element::Utf8(c) if $self.is_noncharacter(c as u32) => {
                $self.parse_error(ParserError::NoncharacterInInputStream);
            }
            _ => {}
        }

        c
    }};
}

// Adds the given character to the current token's value (if applicable)
macro_rules! add_to_token_value {
    ($self:expr, $c:expr) => {
        match &mut $self.current_token {
            Some(Token::CommentToken { value, .. }) => {
                value.push($c);
            }
            _ => {}
        }
    };
}

macro_rules! set_public_identifier {
    ($self:expr, $str:expr) => {
        match &mut $self.current_token {
            Some(Token::DocTypeToken { pub_identifier, .. }) => {
                *pub_identifier = Some($str);
            }
            _ => {}
        }
    };
}

macro_rules! add_public_identifier {
    ($self:expr, $c:expr) => {
        match &mut $self.current_token {
            Some(Token::DocTypeToken {
                pub_identifier: Some(pid),
                ..
            }) => {
                pid.push($c);
            }
            _ => {}
        }
    };
}

macro_rules! set_system_identifier {
    ($self:expr, $str:expr) => {
        match &mut $self.current_token {
            Some(Token::DocTypeToken { sys_identifier, .. }) => {
                *sys_identifier = Some($str);
            }
            _ => {}
        }
    };
}

macro_rules! add_system_identifier {
    ($self:expr, $c:expr) => {
        match &mut $self.current_token {
            Some(Token::DocTypeToken {
                sys_identifier: Some(sid),
                ..
            }) => {
                sid.push($c);
            }
            _ => {}
        }
    };
}

// Adds the given character to the current token's name (if applicable)
macro_rules! add_to_token_name {
    ($self:expr, $c:expr) => {
        match &mut $self.current_token {
            Some(Token::StartTagToken { name, .. }) => {
                name.push($c);
            }
            Some(Token::EndTagToken { name, .. }) => {
                name.push($c);
            }
            Some(Token::DocTypeToken { name, .. }) => {
                // Doctype can have an optional name
                match name {
                    Some(ref mut string) => string.push($c),
                    None => *name = Some($c.to_string()),
                }
            }
            _ => {}
        }
    };
}

// Convert a character to lower case value (assumes character is in A-Z range)
macro_rules! to_lowercase {
    // Converts A-Z to a-z
    ($c:expr) => {
        ((($c) as u8) + 0x20) as char
    };
}

// Emits the current stored token
macro_rules! emit_current_token {
    ($self:expr) => {
        match $self.current_token {
            None => {}
            _ => {
                emit_token!($self, $self.current_token.as_ref().unwrap());
            }
        };
        $self.current_token = None;
    };
}

// Emits the given stored token. It does not have to be stored first.
macro_rules! emit_token {
    ($self:expr, $token:expr) => {
        // Save the start token name if we are pushing it. This helps us in detecting matching tags.
        match $token {
            Token::StartTagToken { name, .. } => {
                $self.last_start_token = String::from(name);
            }
            _ => {}
        }

        // If there is any consumed data, emit this first as a text token
        if $self.has_consumed_data() {
            let value = $self.get_consumed_str().to_string();
            $self.token_queue.push(Token::TextToken { value });
            $self.clear_consume_buffer();
        }

        $self.token_queue.push($token.clone());
    };
}

impl<'a> Tokenizer<'a> {
    // Creates a new tokenizer with the given inputstream and additional options if any
    pub fn new(
        input: &'a mut InputStream,
        opts: Option<Options>,
        error_logger: Rc<RefCell<ErrorLogger>>,
    ) -> Self {
        return Tokenizer {
            stream: input,
            state: opts.as_ref().map_or(State::DataState, |o| o.initial_state),
            last_start_token: opts
                .as_ref()
                .map_or(String::new(), |o| o.last_start_tag.clone()),
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

    pub(crate) fn get_position(&self) -> Position {
        self.stream.position
    }

    // Retrieves the next token from the input stream or Token::EOF when the end is reached
    pub fn next_token(&mut self) -> Result<Token> {
        self.consume_stream()?;

        if self.token_queue.is_empty() {
            return Ok(Token::EofToken);
        }

        Ok(self.token_queue.remove(0))
    }

    pub fn get_error_logger(&self) -> Ref<ErrorLogger> {
        self.error_logger.borrow()
    }

    // Consumes the input stream. Continues until the stream is completed or a token has been generated.
    fn consume_stream(&mut self) -> Result<()> {
        loop {
            // Something is already in the token buffer, so we can return it.
            if !self.token_queue.is_empty() {
                return Ok(());
            }

            match self.state {
                State::DataState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('&') => self.state = State::CharacterReferenceInDataState,
                        Element::Utf8('<') => self.state = State::TagOpenState,
                        Element::Utf8(CHAR_NUL) => {
                            self.consume(c.utf8());
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                        }
                        Element::Eof => {
                            // EOF
                            // if self.has_consumed_data() {
                            //     emit_token!(self, Token::TextToken { value: self.get_consumed_str() });
                            //     self.clear_consume_buffer();
                            // }
                            emit_token!(self, Token::EofToken);
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::CharacterReferenceInDataState => {
                    self.consume_character_reference(None, false);
                    self.state = State::DataState;
                }
                State::RcDataState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('&') => self.state = State::CharacterReferenceInRcDataState,
                        Element::Utf8('<') => self.state = State::RcDataLessThanSignState,
                        Element::Eof => {
                            // if self.has_consumed_data() {
                            //     emit_token!(self, Token::TextToken { value: self.get_consumed_str().clone() });
                            //     self.clear_consume_buffer();
                            // }
                            emit_token!(self, Token::EofToken);
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.consume(CHAR_REPLACEMENT);
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::CharacterReferenceInRcDataState => {
                    // consume character reference
                    self.consume_character_reference(None, false);
                    self.state = State::RcDataState;
                }
                State::RawTextState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('<') => self.state = State::RawTextLessThanSignState,
                        Element::Utf8(CHAR_NUL) => {
                            self.consume(CHAR_REPLACEMENT);
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                        }
                        Element::Eof => {
                            // EOF
                            // if self.has_consumed_data() {
                            //     emit_token!(self, Token::TextToken { value: self.get_consumed_str() });
                            //     self.clear_consume_buffer();
                            // }
                            emit_token!(self, Token::EofToken);
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::ScriptDataState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('<') => self.state = State::ScriptDataLessThenSignState,
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            // if self.has_consumed_data() {
                            //     emit_token!(self, Token::TextToken { value: self.get_consumed_str().clone() });
                            //     self.clear_consume_buffer();
                            // }
                            emit_token!(self, Token::EofToken);
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::PlaintextState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            // if self.has_consumed_data() {
                            //     emit_token!(self, Token::TextToken { value: self.get_consumed_str().clone() });
                            //     self.clear_consume_buffer();
                            // }
                            emit_token!(self, Token::EofToken);
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::TagOpenState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('!') => self.state = State::MarkupDeclarationOpenState,
                        Element::Utf8('/') => self.state = State::EndTagOpenState,
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.current_token = Some(Token::StartTagToken {
                                name: "".into(),
                                is_self_closing: false,
                                attributes: HashMap::new(),
                            });

                            add_to_token_name!(self, to_lowercase!(ch));
                            self.state = State::TagNameState;
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.current_token = Some(Token::StartTagToken {
                                name: "".into(),
                                is_self_closing: false,
                                attributes: HashMap::new(),
                            });

                            add_to_token_name!(self, ch);
                            self.state = State::TagNameState;
                        }
                        Element::Utf8('?') => {
                            self.current_token = Some(Token::CommentToken { value: "".into() });
                            self.parse_error(ParserError::UnexpectedQuestionMarkInsteadOfTagName);
                            self.stream.unread();
                            self.state = State::BogusCommentState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofBeforeTagName);
                            self.consume('<');
                            self.state = State::DataState;
                        }
                        _ => {
                            self.parse_error(ParserError::InvalidFirstCharacterOfTagName);
                            self.stream.unread();
                            self.consume('<');
                            self.state = State::DataState;
                        }
                    }
                }
                State::EndTagOpenState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.current_token = Some(Token::EndTagToken {
                                name: "".into(),
                                is_self_closing: false,
                                attributes: HashMap::new(),
                            });

                            add_to_token_name!(self, to_lowercase!(ch));
                            self.state = State::TagNameState;
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.current_token = Some(Token::EndTagToken {
                                name: "".into(),
                                is_self_closing: false,
                                attributes: HashMap::new(),
                            });

                            add_to_token_name!(self, ch);
                            self.state = State::TagNameState;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingEndTagName);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofBeforeTagName);
                            self.consume('<');
                            self.consume('/');
                            self.state = State::DataState;
                        }
                        _ => {
                            self.parse_error(ParserError::InvalidFirstCharacterOfTagName);

                            self.current_token = Some(Token::CommentToken { value: "".into() });
                            self.stream.unread();
                            self.state = State::BogusCommentState;
                        }
                    }
                }
                State::TagNameState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => self.state = State::BeforeAttributeNameState,
                        Element::Utf8('/') => self.state = State::SelfClosingStartState,
                        Element::Utf8('>') => {
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            add_to_token_name!(self, to_lowercase!(ch))
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            add_to_token_name!(self, CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::DataState;
                        }
                        _ => add_to_token_name!(self, c.utf8()),
                    }
                }
                State::RcDataLessThanSignState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('/') => {
                            self.temporary_buffer.clear();
                            self.state = State::RcDataEndTagOpenState;
                        }
                        _ => {
                            self.consume('<');
                            self.stream.unread();
                            self.state = State::RcDataState;
                        }
                    }
                }
                State::RcDataEndTagOpenState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.current_token = Some(Token::EndTagToken {
                                name: "".into(),
                                is_self_closing: false,
                                attributes: HashMap::new(),
                            });
                            self.temporary_buffer.push(to_lowercase!(ch));
                            self.state = State::RcDataEndTagNameState;
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.current_token = Some(Token::EndTagToken {
                                name: "".into(),
                                is_self_closing: false,
                                attributes: HashMap::new(),
                            });
                            self.temporary_buffer.push(ch);
                            self.state = State::RcDataEndTagNameState;
                        }
                        _ => {
                            self.consume('<');
                            self.consume('/');
                            self.stream.unread();
                            self.state = State::RcDataState;
                        }
                    }
                }
                State::RcDataEndTagNameState => {
                    let c = read_char!(self);

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.state = State::BeforeAttributeNameState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('/') => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.state = State::SelfClosingStartState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('>') => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.set_name_in_current_token(self.temporary_buffer.clone())?;

                                self.last_start_token = String::new();
                                emit_current_token!(self);
                                self.state = State::DataState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.temporary_buffer.push(to_lowercase!(ch));
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.transition_to(State::RcDataState);
                    }
                }
                State::RawTextLessThanSignState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('/') => {
                            self.temporary_buffer.clear();
                            self.state = State::RawTextEndTagOpenState;
                        }
                        _ => {
                            self.consume('<');
                            self.stream.unread();
                            self.state = State::RawTextState;
                        }
                    }
                }
                State::RawTextEndTagOpenState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.current_token = Some(Token::EndTagToken {
                                name: "".into(),
                                is_self_closing: false,
                                attributes: HashMap::new(),
                            });
                            // add_to_token_name!(self, to_lowercase!(ch));
                            self.temporary_buffer.push(to_lowercase!(ch));
                            self.state = State::RawTextEndTagNameState;
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.current_token = Some(Token::EndTagToken {
                                name: "".into(),
                                is_self_closing: false,
                                attributes: HashMap::new(),
                            });
                            // add_to_token_name!(self, ch);
                            self.temporary_buffer.push(ch);
                            self.state = State::RawTextEndTagNameState;
                        }
                        _ => {
                            self.consume('<');
                            self.consume('/');
                            self.stream.unread();
                            self.state = State::RawTextState;
                        }
                    }
                }
                State::RawTextEndTagNameState => {
                    let c = read_char!(self);

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.state = State::BeforeAttributeNameState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('/') => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.state = State::SelfClosingStartState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('>') => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.set_name_in_current_token(self.temporary_buffer.clone())?;
                                self.last_start_token = String::new();
                                emit_current_token!(self);
                                self.state = State::DataState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            // add_to_token_name!(self, to_lowercase!(ch));
                            self.temporary_buffer.push(to_lowercase!(ch));
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            // add_to_token_name!(self, ch);
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.transition_to(State::RawTextState);
                    }
                }
                State::ScriptDataLessThenSignState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('/') => {
                            self.temporary_buffer.clear();
                            self.state = State::ScriptDataEndTagOpenState;
                        }
                        Element::Utf8('!') => {
                            self.consume('<');
                            self.consume('!');
                            self.state = State::ScriptDataEscapeStartState;
                        }
                        _ => {
                            self.consume('<');
                            self.stream.unread();
                            self.state = State::ScriptDataState;
                        }
                    }
                }
                State::ScriptDataEndTagOpenState => {
                    let c = read_char!(self);
                    if c.is_eof() {
                        self.consume('<');
                        self.consume('/');
                        self.stream.unread();
                        self.state = State::ScriptDataState;
                        continue;
                    }

                    if c.utf8().is_ascii_alphabetic() {
                        self.current_token = Some(Token::EndTagToken {
                            name: "".into(),
                            is_self_closing: false,
                            attributes: HashMap::new(),
                        });

                        self.stream.unread();
                        self.state = State::ScriptDataEndTagNameState;
                    } else {
                        self.consume('<');
                        self.consume('/');
                        self.stream.unread();
                        self.state = State::ScriptDataState;
                    }
                }
                State::ScriptDataEndTagNameState => {
                    let c = read_char!(self);

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.state = State::BeforeAttributeNameState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('/') => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.state = State::SelfClosingStartState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('>') => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.set_name_in_current_token(self.temporary_buffer.clone())?;

                                self.last_start_token = String::new();
                                emit_current_token!(self);
                                self.state = State::DataState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.temporary_buffer.push(to_lowercase!(ch));
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.transition_to(State::ScriptDataState);
                    }
                }
                State::ScriptDataEscapeStartState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataEscapeStartDashState;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::ScriptDataState;
                        }
                    }
                }
                State::ScriptDataEscapeStartDashState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataEscapedDashDashState;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::ScriptDataState;
                        }
                    }
                }
                State::ScriptDataEscapedState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataEscapedDashState;
                        }
                        Element::Utf8('<') => {
                            self.state = State::ScriptDataEscapedLessThanSignState;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.consume(c.utf8());
                        }
                    }
                }
                State::ScriptDataEscapedDashState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataEscapedDashDashState;
                        }
                        Element::Utf8('<') => {
                            self.state = State::ScriptDataEscapedLessThanSignState;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataEscapedState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::ScriptDataEscapedState;
                        }
                    }
                }
                State::ScriptDataEscapedDashDashState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                        }
                        Element::Utf8('<') => {
                            self.state = State::ScriptDataEscapedLessThanSignState;
                        }
                        Element::Utf8('>') => {
                            self.consume('>');
                            self.state = State::ScriptDataState;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataEscapedState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::ScriptDataEscapedState;
                        }
                    }
                }
                State::ScriptDataEscapedLessThanSignState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('/') => {
                            self.temporary_buffer.clear();
                            self.state = State::ScriptDataEscapedEndTagOpenState;
                        }
                        _ => {
                            if c.is_utf8() && c.utf8().is_ascii_alphabetic() {
                                self.temporary_buffer.clear();
                                self.consume('<');
                                self.stream.unread();
                                self.state = State::ScriptDataDoubleEscapeStartState;
                                continue;
                            }
                            // anything else
                            self.consume('<');
                            self.stream.unread();
                            self.state = State::ScriptDataEscapedState;
                        }
                    }
                }
                State::ScriptDataEscapedEndTagOpenState => {
                    let c = read_char!(self);

                    if c.is_utf8() && c.utf8().is_ascii_alphabetic() {
                        self.current_token = Some(Token::EndTagToken {
                            name: "".into(),
                            is_self_closing: false,
                            attributes: HashMap::new(),
                        });

                        self.stream.unread();
                        self.state = State::ScriptDataEscapedEndTagNameState;
                        continue;
                    }

                    // anything else
                    self.consume('<');
                    self.consume('/');
                    self.stream.unread();
                    self.state = State::ScriptDataEscapedState;
                }
                State::ScriptDataEscapedEndTagNameState => {
                    let c = read_char!(self);

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.state = State::BeforeAttributeNameState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('/') => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.state = State::SelfClosingStartState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8('>') => {
                            if self.is_appropriate_end_token(&self.temporary_buffer) {
                                self.set_name_in_current_token(self.temporary_buffer.clone())?;
                                self.last_start_token = String::new();
                                emit_current_token!(self);
                                self.state = State::DataState;
                            } else {
                                consume_anything_else = true;
                            }
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.temporary_buffer.push(to_lowercase!(ch));
                        }
                        Element::Utf8(ch @ 'a'..='z') => {
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.transition_to(State::ScriptDataEscapedState);
                    }
                }
                State::ScriptDataDoubleEscapeStartState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE)
                        | Element::Utf8('/')
                        | Element::Utf8('>') => {
                            if self.temporary_buffer == "script" {
                                self.state = State::ScriptDataDoubleEscapedState;
                            } else {
                                self.state = State::ScriptDataEscapedState;
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
                            self.state = State::ScriptDataEscapedState;
                        }
                    }
                }
                State::ScriptDataDoubleEscapedState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataDoubleEscapedDashState;
                        }
                        Element::Utf8('<') => {
                            self.consume('<');
                            self.state = State::ScriptDataDoubleEscapedLessThanSignState;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::DataState;
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::ScriptDataDoubleEscapedDashState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::ScriptDataDoubleEscapedDashDashState;
                            self.consume('-');
                        }
                        Element::Utf8('<') => {
                            self.state = State::ScriptDataDoubleEscapedLessThanSignState;
                            self.consume('<');
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataDoubleEscapedState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.consume(c.utf8());
                            self.state = State::ScriptDataDoubleEscapedState;
                        }
                    }
                }
                State::ScriptDataDoubleEscapedDashDashState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => self.consume('-'),
                        Element::Utf8('<') => {
                            self.consume('<');
                            self.state = State::ScriptDataDoubleEscapedLessThanSignState;
                        }
                        Element::Utf8('>') => {
                            self.consume('>');
                            self.state = State::ScriptDataState;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataDoubleEscapedState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInScriptHtmlCommentLikeText);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.consume(c.utf8());
                            self.state = State::ScriptDataDoubleEscapedState;
                        }
                    }
                }
                State::ScriptDataDoubleEscapedLessThanSignState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('/') => {
                            self.temporary_buffer.clear();
                            self.consume('/');
                            self.state = State::ScriptDataDoubleEscapeEndState;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::ScriptDataDoubleEscapedState;
                        }
                    }
                }
                State::ScriptDataDoubleEscapeEndState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE)
                        | Element::Utf8('/')
                        | Element::Utf8('>') => {
                            if self.temporary_buffer == "script" {
                                self.state = State::ScriptDataEscapedState;
                            } else {
                                self.state = State::ScriptDataDoubleEscapedState;
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
                            self.state = State::ScriptDataDoubleEscapedState;
                        }
                    }
                }
                State::BeforeAttributeNameState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // Ignore character
                        }
                        Element::Utf8('/') | Element::Utf8('>') | Element::Eof => {
                            self.stream.unread();
                            self.state = State::AfterAttributeNameState;
                        }
                        Element::Utf8('=') => {
                            self.parse_error(ParserError::UnexpectedEqualsSignBeforeAttributeName);

                            self.store_and_clear_current_attribute();
                            self.current_attr_name.push(c.utf8());

                            self.state = State::AttributeNameState;
                        }
                        _ => {
                            // Store an existing attribute if any and clear
                            self.store_and_clear_current_attribute();

                            self.stream.unread();
                            self.state = State::AttributeNameState;
                        }
                    }
                }
                State::AttributeNameState => {
                    let c = read_char!(self);
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

                            self.state = State::AfterAttributeNameState
                        }
                        Element::Utf8('=') => {
                            if self.attr_already_exists() {
                                self.parse_error(ParserError::DuplicateAttribute);
                            }
                            self.state = State::BeforeAttributeValueState
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
                State::AfterAttributeNameState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // Ignore
                        }
                        Element::Utf8('/') => self.state = State::SelfClosingStartState,
                        Element::Utf8('=') => self.state = State::BeforeAttributeValueState,
                        Element::Utf8('>') => {
                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.store_and_clear_current_attribute();
                            self.stream.unread();
                            self.state = State::AttributeNameState;
                        }
                    }
                }
                State::BeforeAttributeValueState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // Ignore
                        }
                        Element::Utf8('"') => self.state = State::AttributeValueDoubleQuotedState,
                        Element::Utf8('\'') => {
                            self.state = State::AttributeValueSingleQuotedState;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingAttributeValue);

                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::AttributeValueUnquotedState;
                        }
                    }
                }
                State::AttributeValueDoubleQuotedState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('"') => self.state = State::AfterAttributeValueQuotedState,
                        Element::Utf8('&') => {
                            self.consume_character_reference(Some(Element::Utf8('"')), true);
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.current_attr_value.push(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.current_attr_value.push(c.utf8());
                        }
                    }
                }
                State::AttributeValueSingleQuotedState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('\'') => self.state = State::AfterAttributeValueQuotedState,
                        Element::Utf8('&') => {
                            self.consume_character_reference(Some(Element::Utf8('\'')), true);
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.current_attr_value.push(CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.current_attr_value.push(c.utf8());
                        }
                    }
                }
                State::AttributeValueUnquotedState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            self.state = State::BeforeAttributeNameState;
                        }
                        Element::Utf8('&') => {
                            self.consume_character_reference(Some(Element::Utf8('>')), true);
                        }
                        Element::Utf8('>') => {
                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            emit_current_token!(self);
                            self.state = State::DataState;
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
                            self.state = State::DataState;
                        }
                        _ => {
                            self.current_attr_value.push(c.utf8());
                        }
                    }
                }
                // State::CharacterReferenceInAttributeValueState => {}
                State::AfterAttributeValueQuotedState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => self.state = State::BeforeAttributeNameState,
                        Element::Utf8('/') => self.state = State::SelfClosingStartState,
                        Element::Utf8('>') => {
                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.parse_error(ParserError::MissingWhitespaceBetweenAttributes);
                            self.stream.unread();
                            self.state = State::BeforeAttributeNameState;
                        }
                    }
                }
                State::SelfClosingStartState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('>') => {
                            self.set_is_closing_in_current_token(true);
                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInTag);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.parse_error(ParserError::UnexpectedSolidusInTag);
                            self.stream.unread();
                            self.state = State::BeforeAttributeNameState;
                        }
                    }
                }
                State::BogusCommentState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('>') => {
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            add_to_token_value!(self, CHAR_REPLACEMENT);
                        }
                        _ => {
                            add_to_token_value!(self, c.utf8());
                        }
                    }
                }
                State::MarkupDeclarationOpenState => {
                    if self.stream.look_ahead_slice(2) == "--" {
                        self.current_token = Some(Token::CommentToken { value: "".into() });

                        // Skip the two -- signs
                        self.stream.seek(SeekCur, 2);

                        self.state = State::CommentStartState;
                        continue;
                    }

                    if self.stream.look_ahead_slice(7).to_uppercase() == "DOCTYPE" {
                        self.stream.seek(SeekCur, 7);
                        self.state = State::DocTypeState;
                        continue;
                    }

                    if self.stream.look_ahead_slice(7) == "[CDATA[" {
                        self.stream.seek(SeekCur, 7);

                        // @TODO: If there is an adjusted current node and it is not an element in the HTML namespace,
                        // then switch to the CDATA section state. Otherwise, this is a cdata-in-html-content parse error.
                        // Create a comment token whose data is the "[CDATA[" string. Switch to the bogus comment state.
                        self.parse_error(ParserError::CdataInHtmlContent);
                        self.current_token = Some(Token::CommentToken {
                            value: "[CDATA[".into(),
                        });

                        self.state = State::BogusCommentState;
                        continue;
                    }

                    self.stream.seek(SeekCur, 1);
                    self.parse_error(ParserError::IncorrectlyOpenedComment);
                    self.stream.unread();
                    self.current_token = Some(Token::CommentToken { value: "".into() });

                    self.state = State::BogusCommentState;
                }
                State::CommentStartState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::CommentStartDashState;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptClosingOfEmptyComment);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::CommentState;
                        }
                    }
                }
                State::CommentStartDashState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::CommentEndState;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptClosingOfEmptyComment);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInComment);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            add_to_token_value!(self, '-');
                            self.stream.unread();
                            self.state = State::CommentState;
                        }
                    }
                }
                State::CommentState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('<') => {
                            add_to_token_value!(self, c.utf8());
                            self.state = State::CommentLessThanSignState;
                        }
                        Element::Utf8('-') => self.state = State::CommentEndDashState,
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            add_to_token_value!(self, CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInComment);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            add_to_token_value!(self, c.utf8());
                        }
                    }
                }
                State::CommentLessThanSignState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('!') => {
                            add_to_token_value!(self, c.utf8());
                            self.state = State::CommentLessThanSignBangState;
                        }
                        Element::Utf8('<') => {
                            add_to_token_value!(self, c.utf8());
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::CommentState;
                        }
                    }
                }
                State::CommentLessThanSignBangState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::CommentLessThanSignBangDashState;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::CommentState;
                        }
                    }
                }
                State::CommentLessThanSignBangDashState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::CommentLessThanSignBangDashDashState;
                        }
                        _ => {
                            self.stream.unread();
                            self.state = State::CommentEndDashState;
                        }
                    }
                }
                State::CommentLessThanSignBangDashDashState => {
                    let c = read_char!(self);
                    match c {
                        Element::Eof | Element::Utf8('>') => {
                            self.stream.unread();
                            self.state = State::CommentEndState;
                        }
                        _ => {
                            self.parse_error(ParserError::NestedComment);
                            self.stream.unread();
                            self.state = State::CommentEndState;
                        }
                    }
                }
                State::CommentEndDashState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            self.state = State::CommentEndState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInComment);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            add_to_token_value!(self, '-');
                            self.stream.unread();
                            self.state = State::CommentState;
                        }
                    }
                }
                State::CommentEndState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('>') => {
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Utf8('!') => self.state = State::CommentEndBangState,
                        Element::Utf8('-') => add_to_token_value!(self, '-'),
                        Element::Eof => {
                            self.parse_error(ParserError::EofInComment);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            add_to_token_value!(self, '-');
                            add_to_token_value!(self, '-');
                            self.stream.unread();
                            self.state = State::CommentState;
                        }
                    }
                }
                State::CommentEndBangState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('-') => {
                            add_to_token_value!(self, '-');
                            add_to_token_value!(self, '-');
                            add_to_token_value!(self, '!');

                            self.state = State::CommentEndDashState;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::IncorrectlyClosedComment);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInComment);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            add_to_token_value!(self, '-');
                            add_to_token_value!(self, '-');
                            add_to_token_value!(self, '!');
                            self.stream.unread();
                            self.state = State::CommentState;
                        }
                    }
                }
                State::DocTypeState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => self.state = State::BeforeDocTypeNameState,
                        Element::Utf8('>') => {
                            self.stream.unread();
                            self.state = State::BeforeDocTypeNameState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);

                            emit_token!(
                                self,
                                Token::DocTypeToken {
                                    name: None,
                                    force_quirks: true,
                                    pub_identifier: None,
                                    sys_identifier: None,
                                }
                            );

                            self.state = State::DataState;
                        }
                        _ => {
                            self.parse_error(ParserError::MissingWhitespaceBeforeDoctypeName);
                            self.stream.unread();
                            self.state = State::BeforeDocTypeNameState;
                        }
                    }
                }
                State::BeforeDocTypeNameState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            self.current_token = Some(Token::DocTypeToken {
                                name: None,
                                force_quirks: false,
                                pub_identifier: None,
                                sys_identifier: None,
                            });

                            add_to_token_name!(self, to_lowercase!(ch));
                            self.state = State::DocTypeNameState;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            self.current_token = Some(Token::DocTypeToken {
                                name: None,
                                force_quirks: false,
                                pub_identifier: None,
                                sys_identifier: None,
                            });

                            add_to_token_name!(self, CHAR_REPLACEMENT);
                            self.state = State::DocTypeNameState;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingDoctypeName);
                            emit_token!(
                                self,
                                Token::DocTypeToken {
                                    name: None,
                                    force_quirks: true,
                                    pub_identifier: None,
                                    sys_identifier: None,
                                }
                            );

                            self.state = State::DataState;
                        }

                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);

                            emit_token!(
                                self,
                                Token::DocTypeToken {
                                    name: None,
                                    force_quirks: true,
                                    pub_identifier: None,
                                    sys_identifier: None,
                                }
                            );

                            self.state = State::DataState;
                        }
                        _ => {
                            self.current_token = Some(Token::DocTypeToken {
                                name: None,
                                force_quirks: false,
                                pub_identifier: None,
                                sys_identifier: None,
                            });

                            add_to_token_name!(self, c.utf8());
                            self.state = State::DocTypeNameState;
                        }
                    }
                }
                State::DocTypeNameState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => self.state = State::AfterDocTypeNameState,
                        Element::Utf8('>') => {
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Utf8(ch @ 'A'..='Z') => {
                            add_to_token_name!(self, to_lowercase!(ch))
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            add_to_token_name!(self, CHAR_REPLACEMENT);
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => add_to_token_name!(self, c.utf8()),
                    }
                }
                State::AfterDocTypeNameState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8('>') => {
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.stream.unread();
                            if self.stream.look_ahead_slice(6).to_uppercase() == "PUBLIC" {
                                self.stream.seek(SeekCur, 6);
                                self.state = State::AfterDocTypePublicKeywordState;
                                continue;
                            }
                            if self.stream.look_ahead_slice(6).to_uppercase() == "SYSTEM" {
                                self.stream.seek(SeekCur, 6);
                                self.state = State::AfterDocTypeSystemKeywordState;
                                continue;
                            }
                            // Make sure the parser is on the correct position again since we just
                            // unread the character
                            self.stream.seek(SeekCur, 1);
                            self.parse_error(ParserError::InvalidCharacterSequenceAfterDoctypeName);
                            self.stream.seek(SeekCur, -1);
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocTypeState;
                        }
                    }
                }
                State::AfterDocTypePublicKeywordState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            self.state = State::BeforeDocTypePublicIdentifierState
                        }
                        Element::Utf8('"') => {
                            self.parse_error(
                                ParserError::MissingWhitespaceAfterDoctypePublicKeyword,
                            );
                            set_public_identifier!(self, String::new());
                            self.state = State::DocTypePublicIdentifierDoubleQuotedState;
                        }
                        Element::Utf8('\'') => {
                            self.parse_error(
                                ParserError::MissingWhitespaceAfterDoctypePublicKeyword,
                            );
                            set_public_identifier!(self, String::new());
                            self.state = State::DocTypePublicIdentifierSingleQuotedState;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingDoctypePublicIdentifier);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypePublicIdentifier,
                            );
                            self.stream.unread();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocTypeState;
                        }
                    }
                }
                State::BeforeDocTypePublicIdentifierState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8('"') => {
                            set_public_identifier!(self, String::new());
                            self.state = State::DocTypePublicIdentifierDoubleQuotedState;
                        }
                        Element::Utf8('\'') => {
                            set_public_identifier!(self, String::new());
                            self.state = State::DocTypePublicIdentifierSingleQuotedState;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingDoctypePublicIdentifier);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.stream.unread();
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypePublicIdentifier,
                            );
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocTypeState;
                        }
                    }
                }
                State::DocTypePublicIdentifierDoubleQuotedState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('"') => self.state = State::AfterDoctypePublicIdentifierState,
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            add_public_identifier!(self, CHAR_REPLACEMENT);
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptDoctypePublicIdentifier);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => add_public_identifier!(self, c.utf8()),
                    }
                }
                State::DocTypePublicIdentifierSingleQuotedState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('\'') => {
                            self.state = State::AfterDoctypePublicIdentifierState
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            add_public_identifier!(self, CHAR_REPLACEMENT);
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptDoctypePublicIdentifier);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => add_public_identifier!(self, c.utf8()),
                    }
                }
                State::AfterDoctypePublicIdentifierState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            self.state = State::BetweenDocTypePublicAndSystemIdentifiersState
                        }
                        Element::Utf8('>') => {
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Utf8('"') => {
                            self.parse_error(ParserError::MissingWhitespaceBetweenDoctypePublicAndSystemIdentifiers);
                            set_system_identifier!(self, String::new());
                            self.state = State::DocTypeSystemIdentifierDoubleQuotedState;
                        }
                        Element::Utf8('\'') => {
                            self.parse_error(ParserError::MissingWhitespaceBetweenDoctypePublicAndSystemIdentifiers);
                            set_system_identifier!(self, String::new());
                            self.state = State::DocTypeSystemIdentifierSingleQuotedState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypeSystemIdentifier,
                            );
                            self.stream.unread();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocTypeState;
                        }
                    }
                }
                State::BetweenDocTypePublicAndSystemIdentifiersState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8('>') => {
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Utf8('"') => {
                            set_system_identifier!(self, String::new());
                            self.state = State::DocTypeSystemIdentifierDoubleQuotedState;
                        }
                        Element::Utf8('\'') => {
                            set_system_identifier!(self, String::new());
                            self.state = State::DocTypeSystemIdentifierSingleQuotedState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypeSystemIdentifier,
                            );
                            self.stream.unread();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocTypeState;
                        }
                    }
                }
                State::AfterDocTypeSystemKeywordState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            self.state = State::BeforeDocTypeSystemIdentifierState
                        }
                        Element::Utf8('"') => {
                            self.parse_error(
                                ParserError::MissingWhitespaceAfterDoctypeSystemKeyword,
                            );
                            set_system_identifier!(self, String::new());
                            self.state = State::DocTypeSystemIdentifierDoubleQuotedState;
                        }
                        Element::Utf8('\'') => {
                            self.parse_error(
                                ParserError::MissingWhitespaceAfterDoctypeSystemKeyword,
                            );
                            set_system_identifier!(self, String::new());
                            self.state = State::DocTypeSystemIdentifierSingleQuotedState;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingDoctypeSystemIdentifier);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypeSystemIdentifier,
                            );
                            self.stream.unread();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocTypeState;
                        }
                    }
                }
                State::BeforeDocTypeSystemIdentifierState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8('"') => {
                            set_system_identifier!(self, String::new());
                            self.state = State::DocTypeSystemIdentifierDoubleQuotedState;
                        }
                        Element::Utf8('\'') => {
                            set_system_identifier!(self, String::new());
                            self.state = State::DocTypeSystemIdentifierSingleQuotedState;
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::MissingDoctypeSystemIdentifier);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingQuoteBeforeDoctypeSystemIdentifier,
                            );
                            self.stream.unread();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDocTypeState;
                        }
                    }
                }
                State::DocTypeSystemIdentifierDoubleQuotedState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('"') => self.state = State::AfterDocTypeSystemIdentifierState,
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            add_system_identifier!(self, CHAR_REPLACEMENT);
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptDoctypeSystemIdentifier);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => add_system_identifier!(self, c.utf8()),
                    }
                }
                State::DocTypeSystemIdentifierSingleQuotedState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('\'') => {
                            self.state = State::AfterDocTypeSystemIdentifierState
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter);
                            add_system_identifier!(self, CHAR_REPLACEMENT);
                        }
                        Element::Utf8('>') => {
                            self.parse_error(ParserError::AbruptDoctypeSystemIdentifier);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => add_system_identifier!(self, c.utf8()),
                    }
                }
                State::AfterDocTypeSystemIdentifierState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(CHAR_TAB)
                        | Element::Utf8(CHAR_LF)
                        | Element::Utf8(CHAR_FF)
                        | Element::Utf8(CHAR_SPACE) => {
                            // ignore
                        }
                        Element::Utf8('>') => {
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInDoctype);
                            self.set_quirks_mode(true);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::UnexpectedCharacterAfterDoctypeSystemIdentifier,
                            );
                            self.stream.unread();
                            self.state = State::BogusDocTypeState;
                        }
                    }
                }
                State::BogusDocTypeState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('>') => {
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        Element::Utf8(CHAR_NUL) => {
                            self.parse_error(ParserError::UnexpectedNullCharacter)
                        }
                        Element::Eof => {
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => {
                            // ignore
                        }
                    }
                }
                State::CDataSectionState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(']') => {
                            self.state = State::CDataSectionBracketState;
                        }
                        Element::Eof => {
                            self.parse_error(ParserError::EofInCdata);
                            emit_current_token!(self);
                            self.state = State::DataState;
                        }
                        _ => self.consume(c.utf8()),
                    }
                }
                State::CDataSectionBracketState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(']') => self.state = State::CDataSectionEndState,
                        _ => {
                            self.consume(']');
                            self.stream.unread();
                            self.state = State::CDataSectionState;
                        }
                    }
                }
                State::CDataSectionEndState => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8(']') => self.consume(']'),
                        Element::Utf8('>') => self.state = State::DataState,
                        _ => {
                            self.consume(']');
                            self.consume(']');
                            self.stream.unread();
                            self.state = State::CDataSectionState;
                        }
                    }
                }
                _ => {
                    panic!("state {:?} not implemented", self.state);
                }
            }
        }
    }

    // Consumes the given char
    pub(crate) fn consume(&mut self, c: char) {
        // Add c to the current token data
        self.consumed.push(c)
    }

    fn transition_to(&mut self, state: State) {
        self.consumed.push_str("</");
        self.consumed.push_str(&self.temporary_buffer);
        self.temporary_buffer.clear();
        self.stream.unread();
        self.state = state;
    }

    // Consumes the given string
    pub(crate) fn consume_str(&mut self, s: &str) {
        // Add s to the current token data
        self.consumed.push_str(s);
    }

    // Return true when the given end_token matches the stored start token (ie: 'table' matches when
    // last_start_token = 'table')
    fn is_appropriate_end_token(&self, end_token: &str) -> bool {
        self.last_start_token == end_token
    }

    // Return the consumed string as a String
    pub fn get_consumed_str(&self) -> &str {
        &self.consumed
    }

    // Returns true if there is anything in the consume buffer
    pub fn has_consumed_data(&self) -> bool {
        !self.consumed.is_empty()
    }

    // Clears the current consume buffer
    pub(crate) fn clear_consume_buffer(&mut self) {
        self.consumed.clear()
    }

    // Creates a parser log error message
    pub(crate) fn parse_error(&mut self, message: ParserError) {
        // The previous position is where the error occurred
        let pos = self.stream.get_previous_position();

        self.error_logger
            .borrow_mut()
            .add_error(pos, message.as_str());
    }

    // Set is_closing_tag in current token
    fn set_is_closing_in_current_token(&mut self, is_closing: bool) {
        match &mut self.current_token.as_mut().unwrap() {
            Token::EndTagToken { .. } => {
                self.parse_error(ParserError::EndTagWithTrailingSolidus);
            }
            Token::StartTagToken {
                is_self_closing, ..
            } => {
                *is_self_closing = is_closing;
            }
            _ => {}
        }
    }

    // Set force_quirk mode in current token
    fn set_quirks_mode(&mut self, quirky: bool) {
        if let Token::DocTypeToken { force_quirks, .. } = &mut self.current_token.as_mut().unwrap()
        {
            *force_quirks = quirky;
        }
    }

    // Adds a new attribute to the current token
    fn set_add_attribute_to_current_token(&mut self, name: &str, value: &str) {
        if let Token::StartTagToken { attributes, .. } = &mut self.current_token.as_mut().unwrap() {
            attributes.insert(name.into(), value.into());
        }

        self.current_attr_name.clear()
    }

    // Sets the given name into the current token
    fn set_name_in_current_token(&mut self, new_name: String) -> Result<()> {
        match &mut self.current_token.as_mut().expect("current token") {
            Token::StartTagToken { name, .. } => {
                *name = new_name;
            }
            Token::EndTagToken { name, .. } => {
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

    // This function checks to see if there is already an attribute name like the one in current_attr_name.
    fn attr_already_exists(&mut self) -> bool {
        self.current_attrs.contains_key(&self.current_attr_name)
    }

    // Saves the current attribute name and value onto the current_attrs stack, if there is anything to store
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

    // This method will add current generated attributes to the current (start) token if needed.
    fn add_stored_attributes_to_current_token(&mut self) {
        if self.current_token.is_none() {
            return;
        }
        if self.current_attrs.is_empty() {
            return;
        }

        match self.current_token.as_mut().expect("current token") {
            Token::EndTagToken { .. } => {
                self.parse_error(ParserError::EndTagWithAttributes);
            }
            Token::StartTagToken { attributes, .. } => {
                for (key, value) in &self.current_attrs {
                    attributes.insert(key.clone(), value.clone());
                }
                self.current_attrs = HashMap::new();
            }
            _ => {}
        }
    }
}
