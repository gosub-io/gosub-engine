pub mod state;
pub mod token;

mod character_reference;
mod replacement_tables;

#[cfg(test)]
mod attribute_tests;
#[cfg(test)]
mod test_cases;

use crate::node::HTML_NAMESPACE;
use crate::parser::errors::{ErrorLogger, ParserError};
use crate::tokenizer::state::State;
use crate::tokenizer::token::Token;
use gosub_shared::attributes::Attributes;
use gosub_shared::byte_stream::Character::{Ch, StreamEnd};
use gosub_shared::byte_stream::{ByteStream, Character, Location, Stream};
use gosub_shared::types::Result;
use std::cell::{Ref, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

/// Constants that are not directly captured as visible chars
pub const CHAR_NUL: char = '\u{0000}';
pub const CHAR_TAB: char = '\u{0009}';
pub const CHAR_LF: char = '\u{000A}';
pub const CHAR_CR: char = '\u{000D}';
pub const CHAR_FF: char = '\u{000C}';
pub const CHAR_SPACE: char = '\u{0020}';
pub const CHAR_REPLACEMENT: char = '\u{FFFD}';

/// True for the ASCII bytes that `read_char` passes on without a parse error: printable
/// characters plus TAB, LF, FF and CR. The bulk scans in the states stop at anything else,
/// so NUL, control characters and non-ASCII still go through `read_char`.
#[inline]
fn is_plain_ascii(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | 0x0C | b'\r' | 0x20..=0x7E)
}

/// The tokenizer will read the input stream and emit tokens that can be used by the parser.
pub struct Tokenizer<'tokens> {
    /// HTML character input stream
    pub stream: &'tokens mut ByteStream,
    /// Current state of the tokenizer
    pub state: State,
    /// Current consumed characters for current token
    pub consumed: String,
    /// Current attribute name that we need to store temporary in case we are parsing attributes
    pub current_attr_name: String,
    /// Current attribute value that we need to store temporary in case we are parsing attributes
    pub current_attr_value: String,
    /// Current attributes
    pub current_attrs: Attributes,
    /// Token that is currently in the making (if any)
    pub current_token: Option<Token>,
    /// Temporary buffer
    pub temporary_buffer: String,
    /// Queue of emitted tokens. Needed because we can generate multiple tokens during iteration
    pub token_queue: VecDeque<Token>,
    /// The last emitted start token (or empty if none)
    pub last_start_token: String,
    /// Last token location
    pub last_token_location: Location,
    /// Byte offset of the first character in the consume buffer
    pub consumed_pos: usize,
    /// Last read character
    pub last_char: Character,
    /// Error logger to log errors to
    pub error_logger: Rc<RefCell<ErrorLogger>>,
}

impl Tokenizer<'_> {
    pub(crate) fn insert_tokens_at_queue_start(&mut self, first_tokens: Vec<Token>) {
        for token in first_tokens.into_iter().rev() {
            self.token_queue.push_front(token);
        }
    }
}

/// This struct is a gateway between the parser and the tokenizer. It holds data that can be needed
/// by the tokenizer in certain cases. See <https://github.com/gosub-browser/gosub-engine/issues/230> for
/// more information and how we should refactor this properly.
pub struct ParserData {
    pub adjusted_node_namespace: std::borrow::Cow<'static, str>,
}

impl Default for ParserData {
    fn default() -> Self {
        Self {
            adjusted_node_namespace: std::borrow::Cow::Borrowed(HTML_NAMESPACE),
        }
    }
}

/// Options that can be passed to the tokenizer. Mostly needed when dealing with tests.
pub struct Options {
    /// Sets the initial state of the tokenizer. Normally only needed when dealing with tests
    pub initial_state: State,
    /// Sets the last starting tag in the tokenizer. Normally only needed when dealing with tests
    pub last_start_tag: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            initial_state: State::Data,
            last_start_tag: String::new(),
        }
    }
}

/// Convert a character to lower case value (assumes character is in A-Z range)
macro_rules! to_lowercase {
    ($c:expr) => {
        $c.to_ascii_lowercase()
    };
}

impl<'stream> Tokenizer<'stream> {
    /// Creates a new tokenizer with the given input stream and additional options if any
    #[must_use]
    pub fn new(stream: &'stream mut ByteStream, opts: Option<Options>, error_logger: Rc<RefCell<ErrorLogger>>) -> Self {
        Self {
            stream,
            state: opts.as_ref().map_or(State::Data, |o| o.initial_state),
            last_start_token: opts.map_or(String::new(), |o| o.last_start_tag),
            last_token_location: Location::default(),
            consumed_pos: 0,
            consumed: String::new(),
            current_token: None,
            token_queue: VecDeque::new(),
            current_attr_name: String::new(),
            current_attr_value: String::new(),
            current_attrs: Attributes::new(),
            temporary_buffer: String::new(),
            last_char: StreamEnd,
            error_logger,
        }
    }

    /// Returns the current location in the stream (with line/col number and byte offset)
    #[inline]
    pub(crate) fn get_location(&self) -> Location {
        self.stream.location()
    }

    /// Retrieves the next token from the input stream or `Token::EOF` when the end is reached
    pub fn next_token(&mut self, parser_data: ParserData) -> Result<Token> {
        self.consume_stream(parser_data)?;

        match self.token_queue.pop_front() {
            Some(token) => Ok(token),
            None => Ok(Token::Eof {
                location: self.get_location(),
            }),
        }
    }

    /// Returns the error logger
    #[must_use]
    pub fn get_error_logger(&self) -> Ref<'_, ErrorLogger> {
        self.error_logger.borrow()
    }

    /// Sets the tokenizer state to a new state
    pub(crate) fn set_state(&mut self, state: State) {
        self.state = state;
    }

    /// Consumes the input stream. Continues until the stream is completed or a token has been generated.
    fn consume_stream(&mut self, parser_data: ParserData) -> Result<()> {
        loop {
            // Something is already in the token buffer, so we can return it.
            if !self.token_queue.is_empty() {
                return Ok(());
            }

            // Text starts in one of these states. The states after a '<' or '&' can still end
            // up adding to the text, but then it starts at the '<' or '&' itself.
            if matches!(
                self.state,
                State::Data
                    | State::RCDATA
                    | State::RAWTEXT
                    | State::ScriptData
                    | State::PLAINTEXT
                    | State::CDATASection
            ) && !self.has_consumed_data()
            {
                self.consumed_pos = self.stream.tell_bytes();
            }

            match self.state {
                State::Data => {
                    let run = self
                        .stream
                        .scan_ascii_while(|b| is_plain_ascii(b) && b != b'&' && b != b'<');
                    self.consumed.push_str(run);
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('&') => self.state = State::CharacterReferenceInData,
                        Ch('<') => {
                            self.state = {
                                self.last_token_location = self.stream.location_at(pos);
                                State::TagOpen
                            }
                        }
                        Ch(CHAR_NUL) => {
                            self.consume(c.into());
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                        }
                        StreamEnd => self.emit_token(Token::Eof {
                            location: self.get_location(),
                        }),
                        _ => self.consume(c.into()),
                    }
                }
                State::CharacterReferenceInData => {
                    self.consume_character_reference(false);
                    self.state = State::Data;
                }
                State::RCDATA => {
                    let run = self
                        .stream
                        .scan_ascii_while(|b| is_plain_ascii(b) && b != b'&' && b != b'<');
                    self.consumed.push_str(run);
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('&') => self.state = State::CharacterReferenceInRcData,
                        Ch('<') => self.state = State::RCDATALessThanSign,
                        StreamEnd => self.emit_token(Token::Eof {
                            location: self.get_location(),
                        }),
                        Ch(CHAR_NUL) => {
                            self.consume(CHAR_REPLACEMENT);
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                        }
                        _ => self.consume(c.into()),
                    }
                }
                State::CharacterReferenceInRcData => {
                    // consume character reference
                    self.consume_character_reference(false);
                    self.state = State::RCDATA;
                }
                State::RAWTEXT => {
                    let run = self.stream.scan_ascii_while(|b| is_plain_ascii(b) && b != b'<');
                    self.consumed.push_str(run);
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('<') => self.state = State::RAWTEXTLessThanSign,
                        Ch(CHAR_NUL) => {
                            self.consume(CHAR_REPLACEMENT);
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                        }
                        StreamEnd => self.emit_token(Token::Eof {
                            location: self.get_location(),
                        }),
                        _ => self.consume(c.into()),
                    }
                }
                State::ScriptData => {
                    let run = self.stream.scan_ascii_while(|b| is_plain_ascii(b) && b != b'<');
                    self.consumed.push_str(run);
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('<') => self.state = State::ScriptDataLessThenSign,
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        StreamEnd => self.emit_token(Token::Eof {
                            location: self.get_location(),
                        }),
                        _ => self.consume(c.into()),
                    }
                }
                State::PLAINTEXT => {
                    let run = self.stream.scan_ascii_while(is_plain_ascii);
                    self.consumed.push_str(run);
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        StreamEnd => self.emit_token(Token::Eof {
                            location: self.get_location(),
                        }),
                        _ => self.consume(c.into()),
                    }
                }
                State::TagOpen => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('!') => self.state = State::MarkupDeclarationOpen,
                        Ch('/') => self.state = State::EndTagOpen,
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::StartTag {
                                name: String::new(),
                                is_self_closing: false,
                                attributes: Attributes::new(),
                                location: self.last_token_location,
                            });
                            self.stream_prev();
                            self.state = State::TagName;
                        }
                        Ch('?') => {
                            self.current_token = Some(Token::Comment {
                                comment: String::new(),
                                location: self.last_token_location,
                            });
                            self.parse_error_at(ParserError::UnexpectedQuestionMarkInsteadOfTagName, pos);
                            self.stream_prev();
                            self.state = State::BogusComment;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofBeforeTagName, pos);
                            self.consume('<');
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error_at(ParserError::InvalidFirstCharacterOfTagName, pos);
                            self.consume('<');
                            self.stream_prev();
                            self.state = State::Data;
                        }
                    }
                }
                State::EndTagOpen => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::EndTag {
                                name: String::new(),
                                location: self.last_token_location,
                            });
                            self.stream_prev();
                            self.state = State::TagName;
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::MissingEndTagName, pos);
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofBeforeTagName, pos);
                            self.consume('<');
                            self.consume('/');
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error_at(ParserError::InvalidFirstCharacterOfTagName, pos);
                            self.current_token = Some(Token::Comment {
                                comment: String::new(),
                                location: self.last_token_location,
                            });
                            self.stream_prev();
                            self.state = State::BogusComment;
                        }
                    }
                }
                State::TagName => {
                    if let Some(Token::StartTag { name, .. } | Token::EndTag { name, .. }) = &mut self.current_token {
                        let start = name.len();
                        name.push_str(self.stream.scan_ascii_while(|b| {
                            is_plain_ascii(b) && !matches!(b, b'\t' | b'\n' | 0x0C | b' ' | b'/' | b'>')
                        }));
                        name[start..].make_ascii_lowercase();
                    }
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            self.state = State::BeforeAttributeName;
                        }
                        Ch('/') => self.state = State::SelfClosingStart,
                        Ch('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Ch(ch @ 'A'..='Z') => self.add_to_token_name(to_lowercase!(ch)),
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.add_to_token_name(CHAR_REPLACEMENT);
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInTag, pos);
                            self.state = State::Data;
                        }
                        _ => self.add_to_token_name(c.into()),
                    }
                }
                State::RCDATALessThanSign => {
                    let c = self.read_char();
                    if let Ch('/') = c {
                        self.temporary_buffer.clear();
                        self.state = State::RCDATAEndTagOpen;
                    } else {
                        self.consume('<');
                        self.stream_prev();
                        self.state = State::RCDATA;
                    }
                }
                State::RCDATAEndTagOpen => {
                    let c = self.read_char();
                    match c {
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::EndTag {
                                name: String::new(),
                                location: self.last_token_location,
                            });
                            self.stream_prev();
                            self.state = State::RCDATAEndTagName;
                        }
                        _ => {
                            self.consume('<');
                            self.consume('/');
                            self.stream_prev();
                            self.state = State::RCDATA;
                        }
                    }
                }
                State::RCDATAEndTagName => {
                    let c = self.read_char();

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
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
                        Ch('/') => {
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
                        Ch('>') => {
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
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.add_to_token_name(to_lowercase!(ch));
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.stream_prev();
                        self.transition_to(State::RCDATA);
                    }
                }
                State::RAWTEXTLessThanSign => {
                    let c = self.read_char();
                    if let Ch('/') = c {
                        self.temporary_buffer.clear();
                        self.state = State::RAWTEXTEndTagOpen;
                    } else {
                        self.consume('<');
                        self.stream_prev();
                        self.state = State::RAWTEXT;
                    }
                }
                State::RAWTEXTEndTagOpen => {
                    let c = self.read_char();
                    match c {
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::EndTag {
                                name: String::new(),
                                location: self.last_token_location,
                            });
                            self.stream_prev();
                            self.state = State::RAWTEXTEndTagName;
                        }
                        _ => {
                            self.consume('<');
                            self.consume('/');
                            self.stream_prev();
                            self.state = State::RAWTEXT;
                        }
                    }
                }
                State::RAWTEXTEndTagName => {
                    let c = self.read_char();

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
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
                        Ch('/') => {
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
                        Ch('>') => {
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
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.add_to_token_name(to_lowercase!(ch));
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.stream_prev();
                        self.transition_to(State::RAWTEXT);
                    }
                }
                State::ScriptDataLessThenSign => {
                    let c = self.read_char();
                    match c {
                        Ch('/') => {
                            self.temporary_buffer.clear();
                            self.state = State::ScriptDataEndTagOpen;
                        }
                        Ch('!') => {
                            self.consume('<');
                            self.consume('!');
                            self.state = State::ScriptDataEscapeStart;
                        }
                        _ => {
                            self.consume('<');
                            self.stream_prev();
                            self.state = State::ScriptData;
                        }
                    }
                }
                State::ScriptDataEndTagOpen => {
                    let c = self.read_char();
                    match c {
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::EndTag {
                                name: format!("{}", to_lowercase!(ch)),
                                location: self.last_token_location,
                            });

                            self.temporary_buffer.push(ch);

                            self.state = State::ScriptDataEndTagName;
                        }
                        _ => {
                            self.consume('<');
                            self.consume('/');
                            self.stream_prev();
                            self.state = State::ScriptData;
                        }
                    }
                }
                State::ScriptDataEndTagName => {
                    let c = self.read_char();

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
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
                        Ch('/') => {
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
                        Ch('>') => {
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
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.add_to_token_name(to_lowercase!(ch));
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.stream_prev();
                        self.transition_to(State::ScriptData);
                    }
                }
                State::ScriptDataEscapeStart => {
                    let c = self.read_char();
                    if let Ch('-') = c {
                        self.consume('-');
                        self.state = State::ScriptDataEscapeStartDash;
                    } else {
                        self.stream_prev();
                        self.state = State::ScriptData;
                    }
                }
                State::ScriptDataEscapeStartDash => {
                    let c = self.read_char();
                    if let Ch('-') = c {
                        self.consume('-');
                        self.state = State::ScriptDataEscapedDashDash;
                    } else {
                        self.stream_prev();
                        self.state = State::ScriptData;
                    }
                }
                State::ScriptDataEscaped => {
                    let run = self
                        .stream
                        .scan_ascii_while(|b| is_plain_ascii(b) && b != b'-' && b != b'<');
                    self.consumed.push_str(run);
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataEscapedDash;
                        }
                        Ch('<') => {
                            self.state = State::ScriptDataEscapedLessThanSign;
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInScriptHtmlCommentLikeText, pos);
                            self.state = State::Data;
                        }
                        _ => {
                            self.consume(c.into());
                        }
                    }
                }
                State::ScriptDataEscapedDash => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataEscapedDashDash;
                        }
                        Ch('<') => {
                            self.state = State::ScriptDataEscapedLessThanSign;
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataEscaped;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInScriptHtmlCommentLikeText, pos);
                            self.state = State::Data;
                        }
                        _ => {
                            self.consume(c.into());
                            self.state = State::ScriptDataEscaped;
                        }
                    }
                }
                State::ScriptDataEscapedDashDash => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('-') => {
                            self.consume('-');
                        }
                        Ch('<') => {
                            self.state = State::ScriptDataEscapedLessThanSign;
                        }
                        Ch('>') => {
                            self.consume('>');
                            self.state = State::ScriptData;
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataEscaped;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInScriptHtmlCommentLikeText, pos);
                            self.state = State::Data;
                        }
                        _ => {
                            self.consume(c.into());
                            self.state = State::ScriptDataEscaped;
                        }
                    }
                }
                State::ScriptDataEscapedLessThanSign => {
                    let c = self.read_char();
                    match c {
                        Ch('/') => {
                            self.temporary_buffer.clear();
                            self.state = State::ScriptDataEscapedEndTagOpen;
                        }
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.temporary_buffer.clear();
                            self.consume('<');
                            self.stream_prev();
                            self.state = State::ScriptDataDoubleEscapeStart;
                        }
                        _ => {
                            // anything else
                            self.consume('<');
                            self.stream_prev();
                            self.state = State::ScriptDataEscaped;
                        }
                    }
                }
                State::ScriptDataEscapedEndTagOpen => {
                    let c = self.read_char();
                    match c {
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.current_token = Some(Token::EndTag {
                                name: String::new(),
                                location: self.last_token_location,
                            });

                            self.stream_prev();
                            self.state = State::ScriptDataEscapedEndTagName;
                        }
                        _ => {
                            self.consume('<');
                            self.consume('/');
                            self.stream_prev();
                            self.state = State::ScriptDataEscaped;
                        }
                    }
                }
                State::ScriptDataEscapedEndTagName => {
                    let c = self.read_char();

                    // we use this flag because a lot of matches will actually do the same thing
                    let mut consume_anything_else = false;

                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
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
                        Ch('/') => {
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
                        Ch('>') => {
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
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.add_to_token_name(to_lowercase!(ch));
                            self.temporary_buffer.push(ch);
                        }
                        _ => {
                            consume_anything_else = true;
                        }
                    }

                    if consume_anything_else {
                        self.stream_prev();
                        self.transition_to(State::ScriptDataEscaped);
                    }
                }
                State::ScriptDataDoubleEscapeStart => {
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE | '/' | '>') => {
                            if self.temporary_buffer == "script" {
                                self.state = State::ScriptDataDoubleEscaped;
                            } else {
                                self.state = State::ScriptDataEscaped;
                            }
                            self.consume(c.into());
                        }
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.temporary_buffer.push(to_lowercase!(ch));
                            self.consume(ch);
                        }
                        _ => {
                            self.stream_prev();
                            self.state = State::ScriptDataEscaped;
                        }
                    }
                }
                State::ScriptDataDoubleEscaped => {
                    let run = self
                        .stream
                        .scan_ascii_while(|b| is_plain_ascii(b) && b != b'-' && b != b'<');
                    self.consumed.push_str(run);
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('-') => {
                            self.consume('-');
                            self.state = State::ScriptDataDoubleEscapedDash;
                        }
                        Ch('<') => {
                            self.consume('<');
                            self.state = State::ScriptDataDoubleEscapedLessThanSign;
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.consume(CHAR_REPLACEMENT);
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInScriptHtmlCommentLikeText, pos);
                            self.state = State::Data;
                        }
                        _ => self.consume(c.into()),
                    }
                }
                State::ScriptDataDoubleEscapedDash => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('-') => {
                            self.state = State::ScriptDataDoubleEscapedDashDash;
                            self.consume('-');
                        }
                        Ch('<') => {
                            self.state = State::ScriptDataDoubleEscapedLessThanSign;
                            self.consume('<');
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInScriptHtmlCommentLikeText, pos);
                            self.state = State::Data;
                        }
                        _ => {
                            self.consume(c.into());
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                    }
                }
                State::ScriptDataDoubleEscapedDashDash => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('-') => self.consume('-'),
                        Ch('<') => {
                            self.consume('<');
                            self.state = State::ScriptDataDoubleEscapedLessThanSign;
                        }
                        Ch('>') => {
                            self.consume('>');
                            self.state = State::ScriptData;
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.consume(CHAR_REPLACEMENT);
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInScriptHtmlCommentLikeText, pos);
                            self.state = State::Data;
                        }
                        _ => {
                            self.consume(c.into());
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                    }
                }
                State::ScriptDataDoubleEscapedLessThanSign => {
                    let c = self.read_char();
                    if let Ch('/') = c {
                        self.temporary_buffer.clear();
                        self.consume('/');
                        self.state = State::ScriptDataDoubleEscapeEnd;
                    } else {
                        self.stream_prev();
                        self.state = State::ScriptDataDoubleEscaped;
                    }
                }
                State::ScriptDataDoubleEscapeEnd => {
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE | '/' | '>') => {
                            if self.temporary_buffer == "script" {
                                self.state = State::ScriptDataEscaped;
                            } else {
                                self.state = State::ScriptDataDoubleEscaped;
                            }
                            self.consume(c.into());
                        }
                        Ch(ch) if ch.is_ascii_alphabetic() => {
                            self.temporary_buffer.push(to_lowercase!(ch));
                            self.consume(ch);
                        }
                        _ => {
                            self.stream_prev();
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                    }
                }
                State::BeforeAttributeName => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            // Ignore character
                        }
                        Ch('/' | '>') | StreamEnd => {
                            self.stream_prev();
                            self.state = State::AfterAttributeName;
                        }
                        Ch('=') => {
                            self.parse_error_at(ParserError::UnexpectedEqualsSignBeforeAttributeName, pos);

                            self.store_and_clear_current_attribute();
                            self.current_attr_name.push(c.into());

                            self.state = State::AttributeName;
                        }
                        _ => {
                            // Store an existing attribute if any and clear
                            self.store_and_clear_current_attribute();

                            self.stream_prev();
                            self.state = State::AttributeName;
                        }
                    }
                }
                State::AttributeName => {
                    let start = self.current_attr_name.len();
                    self.current_attr_name.push_str(self.stream.scan_ascii_while(|b| {
                        is_plain_ascii(b)
                            && !matches!(
                                b,
                                b'\t' | b'\n' | 0x0C | b' ' | b'/' | b'>' | b'=' | b'"' | b'\'' | b'<'
                            )
                    }));
                    self.current_attr_name[start..].make_ascii_lowercase();
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE | '/' | '>') => {
                            if self.attr_already_exists() {
                                self.parse_error_at(ParserError::DuplicateAttribute, pos);
                            }
                            self.stream_prev();

                            self.state = State::AfterAttributeName;
                        }
                        StreamEnd => {
                            if self.attr_already_exists() {
                                self.parse_error_at(ParserError::DuplicateAttribute, pos);
                            }
                            self.state = State::AfterAttributeName;
                        }
                        Ch('=') => {
                            if self.attr_already_exists() {
                                self.parse_error_at(ParserError::DuplicateAttribute, pos);
                            }
                            self.state = State::BeforeAttributeValue;
                        }
                        Ch(ch @ 'A'..='Z') => {
                            self.current_attr_name.push(to_lowercase!(ch));
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.current_attr_name.push(CHAR_REPLACEMENT);
                        }
                        Ch('"' | '\'' | '<') => {
                            self.parse_error_at(ParserError::UnexpectedCharacterInAttributeName, pos);
                            self.current_attr_name.push(c.into());
                        }
                        _ => self.current_attr_name.push(c.into()),
                    }
                }
                State::AfterAttributeName => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            // Ignore
                        }
                        Ch('/') => self.state = State::SelfClosingStart,
                        Ch('=') => self.state = State::BeforeAttributeValue,
                        Ch('>') => {
                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInTag, pos);
                            self.clear_attribute_state();
                            self.state = State::Data;
                        }
                        _ => {
                            self.store_and_clear_current_attribute();
                            self.stream_prev();
                            self.state = State::AttributeName;
                        }
                    }
                }
                State::BeforeAttributeValue => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            // Ignore
                        }
                        Ch('"') => self.state = State::AttributeValueDoubleQuoted,
                        Ch('\'') => {
                            self.state = State::AttributeValueSingleQuoted;
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::MissingAttributeValue, pos);

                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInTag, pos);
                            self.clear_attribute_state();
                            self.state = State::Data;
                        }
                        _ => {
                            self.stream_prev();
                            self.state = State::AttributeValueUnquoted;
                        }
                    }
                }
                State::AttributeValueDoubleQuoted => {
                    let run = self
                        .stream
                        .scan_ascii_while(|b| is_plain_ascii(b) && b != b'"' && b != b'&');
                    self.current_attr_value.push_str(run);
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('"') => self.state = State::AfterAttributeValueQuoted,
                        Ch('&') => {
                            self.consume_character_reference(true);
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.current_attr_value.push(CHAR_REPLACEMENT);
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInTag, pos);
                            self.clear_attribute_state();
                            self.state = State::Data;
                        }
                        _ => {
                            self.current_attr_value.push(c.into());
                        }
                    }
                }
                State::AttributeValueSingleQuoted => {
                    let run = self
                        .stream
                        .scan_ascii_while(|b| is_plain_ascii(b) && b != b'\'' && b != b'&');
                    self.current_attr_value.push_str(run);
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('\'') => self.state = State::AfterAttributeValueQuoted,
                        Ch('&') => {
                            self.consume_character_reference(true);
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.current_attr_value.push(CHAR_REPLACEMENT);
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInTag, pos);
                            self.clear_attribute_state();
                            self.state = State::Data;
                        }
                        _ => {
                            self.current_attr_value.push(c.into());
                        }
                    }
                }
                State::AttributeValueUnquoted => {
                    let run = self.stream.scan_ascii_while(|b| {
                        is_plain_ascii(b)
                            && !matches!(
                                b,
                                b'\t' | b'\n' | 0x0C | b' ' | b'&' | b'>' | b'"' | b'\'' | b'<' | b'=' | b'`'
                            )
                    });
                    self.current_attr_value.push_str(run);
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            self.state = State::BeforeAttributeName;
                        }
                        Ch('&') => {
                            self.consume_character_reference(true);
                        }
                        Ch('>') => {
                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.current_attr_value.push(CHAR_REPLACEMENT);
                        }
                        Ch('"' | '\'' | '<' | '=' | '`') => {
                            self.parse_error_at(ParserError::UnexpectedCharacterInUnquotedAttributeValue, pos);
                            self.current_attr_value.push(c.into());
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInTag, pos);
                            self.clear_attribute_state();
                            self.state = State::Data;
                        }
                        _ => {
                            self.current_attr_value.push(c.into());
                        }
                    }
                }
                State::AfterAttributeValueQuoted => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            self.state = State::BeforeAttributeName;
                        }
                        Ch('/') => self.state = State::SelfClosingStart,
                        Ch('>') => {
                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInTag, pos);
                            self.clear_attribute_state();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error_at(ParserError::MissingWhitespaceBetweenAttributes, pos);
                            self.stream_prev();
                            self.state = State::BeforeAttributeName;
                        }
                    }
                }
                State::SelfClosingStart => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('>') => {
                            self.set_is_closing_in_current_token(true);

                            self.state = State::Data;

                            self.store_and_clear_current_attribute();
                            self.add_stored_attributes_to_current_token();
                            self.emit_current_token();
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInTag, pos);
                            self.clear_attribute_state();
                            self.state = State::Data;
                        }
                        _ => {
                            self.stream_prev();
                            self.parse_error_at(ParserError::UnexpectedSolidusInTag, pos);
                            self.state = State::BeforeAttributeName;
                        }
                    }
                }
                State::BogusComment => {
                    if let Some(Token::Comment { comment, .. }) = &mut self.current_token {
                        comment.push_str(self.stream.scan_ascii_while(|b| is_plain_ascii(b) && b != b'>'));
                    }
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('>') | StreamEnd => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.add_to_token_value(CHAR_REPLACEMENT);
                        }
                        _ => {
                            self.add_to_token_value(c.into());
                        }
                    }
                }
                State::MarkupDeclarationOpen => {
                    if Character::slice_to_string(self.stream.get_slice(2)) == "--" {
                        self.current_token = Some(Token::Comment {
                            comment: String::new(),
                            location: self.last_token_location,
                        });

                        // Skip the two -- signs
                        self.stream_next_n(2);

                        self.state = State::CommentStart;
                        continue;
                    }

                    if self.lookahead_eq_ignore_ascii_case("DOCTYPE") {
                        self.stream_next_n(7);
                        self.state = State::DOCTYPE;
                        continue;
                    }

                    if Character::slice_to_string(self.stream.get_slice(7)) == "[CDATA[" {
                        self.stream_next_n(6);
                        let pos = self.stream.tell_bytes();
                        self.stream_next_n(1);

                        if parser_data.adjusted_node_namespace != HTML_NAMESPACE {
                            self.state = State::CDATASection;
                            continue;
                        }

                        self.parse_error_at(ParserError::CdataInHtmlContent, pos);
                        self.current_token = Some(Token::Comment {
                            comment: "[CDATA[".into(),
                            location: self.last_token_location,
                        });

                        self.state = State::BogusComment;
                        continue;
                    }

                    self.parse_error(ParserError::IncorrectlyOpenedComment, self.get_location());
                    self.current_token = Some(Token::Comment {
                        comment: String::new(),
                        location: self.last_token_location,
                    });

                    self.state = State::BogusComment;
                }
                State::CommentStart => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('-') => {
                            self.state = State::CommentStartDash;
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::AbruptClosingOfEmptyComment, pos);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.stream_prev();
                            self.state = State::Comment;
                        }
                    }
                }
                State::CommentStartDash => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('-') => {
                            self.state = State::CommentEnd;
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::AbruptClosingOfEmptyComment, pos);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInComment, pos);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.add_to_token_value('-');
                            self.stream_prev();
                            self.state = State::Comment;
                        }
                    }
                }
                State::Comment => {
                    if let Some(Token::Comment { comment, .. }) = &mut self.current_token {
                        comment.push_str(
                            self.stream
                                .scan_ascii_while(|b| is_plain_ascii(b) && b != b'<' && b != b'-'),
                        );
                    }
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('<') => {
                            self.add_to_token_value(c.into());
                            self.state = State::CommentLessThanSign;
                        }
                        Ch('-') => self.state = State::CommentEndDash,
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.add_to_token_value(CHAR_REPLACEMENT);
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInComment, pos);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.add_to_token_value(c.into());
                        }
                    }
                }
                State::CommentLessThanSign => {
                    let c = self.read_char();
                    match c {
                        Ch('!') => {
                            self.add_to_token_value(c.into());
                            self.state = State::CommentLessThanSignBang;
                        }
                        Ch('<') => {
                            self.add_to_token_value(c.into());
                        }
                        _ => {
                            self.stream_prev();
                            self.state = State::Comment;
                        }
                    }
                }
                State::CommentLessThanSignBang => {
                    let c = self.read_char();
                    if let Ch('-') = c {
                        self.state = State::CommentLessThanSignBangDash;
                    } else {
                        self.stream_prev();
                        self.state = State::Comment;
                    }
                }
                State::CommentLessThanSignBangDash => {
                    let c = self.read_char();
                    if let Ch('-') = c {
                        self.state = State::CommentLessThanSignBangDashDash;
                    } else {
                        self.stream_prev();
                        self.state = State::CommentEndDash;
                    }
                }
                State::CommentLessThanSignBangDashDash => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        StreamEnd => {
                            self.state = State::CommentEnd;
                        }
                        Ch('>') => {
                            self.stream_prev();
                            self.state = State::CommentEnd;
                        }
                        _ => {
                            self.parse_error_at(ParserError::NestedComment, pos);
                            self.stream_prev();
                            self.state = State::CommentEnd;
                        }
                    }
                }
                State::CommentEndDash => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('-') => {
                            self.state = State::CommentEnd;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInComment, pos);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.add_to_token_value('-');
                            self.stream_prev();
                            self.state = State::Comment;
                        }
                    }
                }
                State::CommentEnd => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Ch('!') => self.state = State::CommentEndBang,
                        Ch('-') => self.add_to_token_value('-'),
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInComment, pos);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.add_to_token_value('-');
                            self.add_to_token_value('-');
                            self.stream_prev();
                            self.state = State::Comment;
                        }
                    }
                }
                State::CommentEndBang => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('-') => {
                            self.add_to_token_value('-');
                            self.add_to_token_value('-');
                            self.add_to_token_value('!');

                            self.state = State::CommentEndDash;
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::IncorrectlyClosedComment, pos);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInComment, pos);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.add_to_token_value('-');
                            self.add_to_token_value('-');
                            self.add_to_token_value('!');
                            self.stream_prev();
                            self.state = State::Comment;
                        }
                    }
                }
                State::DOCTYPE => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            self.state = State::BeforeDOCTYPEName;
                        }
                        Ch('>') => {
                            self.stream_prev();
                            self.state = State::BeforeDOCTYPEName;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);

                            self.emit_token(Token::DocType {
                                name: None,
                                force_quirks: true,
                                pub_identifier: None,
                                sys_identifier: None,
                                location: self.get_location(),
                            });

                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error_at(ParserError::MissingWhitespaceBeforeDoctypeName, pos);
                            self.stream_prev();
                            self.state = State::BeforeDOCTYPEName;
                        }
                    }
                }
                State::BeforeDOCTYPEName => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            // ignore
                        }
                        Ch(ch @ 'A'..='Z') => {
                            self.current_token = Some(Token::DocType {
                                name: None,
                                force_quirks: false,
                                pub_identifier: None,
                                sys_identifier: None,
                                location: self.last_token_location,
                            });

                            self.add_to_token_name(to_lowercase!(ch));
                            self.state = State::DOCTYPEName;
                        }
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.current_token = Some(Token::DocType {
                                name: None,
                                force_quirks: false,
                                pub_identifier: None,
                                sys_identifier: None,
                                location: self.last_token_location,
                            });

                            self.add_to_token_name(CHAR_REPLACEMENT);
                            self.state = State::DOCTYPEName;
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::MissingDoctypeName, pos);
                            self.emit_token(Token::DocType {
                                name: None,
                                force_quirks: true,
                                pub_identifier: None,
                                sys_identifier: None,
                                location: self.last_token_location,
                            });

                            self.state = State::Data;
                        }

                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);

                            self.emit_token(Token::DocType {
                                name: None,
                                force_quirks: true,
                                pub_identifier: None,
                                sys_identifier: None,
                                location: self.last_token_location,
                            });

                            self.state = State::Data;
                        }
                        _ => {
                            self.current_token = Some(Token::DocType {
                                name: None,
                                force_quirks: false,
                                pub_identifier: None,
                                sys_identifier: None,
                                location: self.last_token_location,
                            });

                            self.add_to_token_name(c.into());
                            self.state = State::DOCTYPEName;
                        }
                    }
                }
                State::DOCTYPEName => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            self.state = State::AfterDOCTYPEName;
                        }
                        Ch('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Ch(ch @ 'A'..='Z') => self.add_to_token_name(to_lowercase!(ch)),
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.add_to_token_name(CHAR_REPLACEMENT);
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => self.add_to_token_name(c.into()),
                    }
                }
                State::AfterDOCTYPEName => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            // ignore
                        }
                        Ch('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.stream_prev();
                            if self.lookahead_eq_ignore_ascii_case("PUBLIC") {
                                self.stream_next_n(6);
                                self.state = State::AfterDOCTYPEPublicKeyword;
                                continue;
                            }
                            if self.lookahead_eq_ignore_ascii_case("SYSTEM") {
                                self.stream_next_n(6);
                                self.state = State::AfterDOCTYPESystemKeyword;
                                continue;
                            }
                            // Make sure the parser is on the correct position again since we just
                            // unread the character
                            let pos = self.stream.tell_bytes();
                            self.stream_next_n(1);
                            self.parse_error_at(ParserError::InvalidCharacterSequenceAfterDoctypeName, pos);
                            self.stream_prev();
                            self.set_quirks_mode(true);
                            self.stream_prev();
                            self.state = State::BogusDOCTYPE;
                        }
                    }
                }
                State::AfterDOCTYPEPublicKeyword => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            self.state = State::BeforeDOCTYPEPublicIdentifier;
                        }
                        Ch('"') => {
                            self.parse_error_at(ParserError::MissingWhitespaceAfterDoctypePublicKeyword, pos);
                            self.set_public_identifier(String::new());
                            self.state = State::DOCTYPEPublicIdentifierDoubleQuoted;
                        }
                        Ch('\'') => {
                            self.parse_error_at(ParserError::MissingWhitespaceAfterDoctypePublicKeyword, pos);
                            self.set_public_identifier(String::new());
                            self.state = State::DOCTYPEPublicIdentifierSingleQuoted;
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::MissingDoctypePublicIdentifier, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error_at(ParserError::MissingQuoteBeforeDoctypePublicIdentifier, pos);
                            self.stream_prev();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDOCTYPE;
                        }
                    }
                }
                State::BeforeDOCTYPEPublicIdentifier => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            // ignore
                        }
                        Ch('"') => {
                            self.set_public_identifier(String::new());
                            self.state = State::DOCTYPEPublicIdentifierDoubleQuoted;
                        }
                        Ch('\'') => {
                            self.set_public_identifier(String::new());
                            self.state = State::DOCTYPEPublicIdentifierSingleQuoted;
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::MissingDoctypePublicIdentifier, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.stream_prev();
                            self.parse_error_at(ParserError::MissingQuoteBeforeDoctypePublicIdentifier, pos);
                            self.set_quirks_mode(true);
                            self.state = State::BogusDOCTYPE;
                        }
                    }
                }
                State::DOCTYPEPublicIdentifierDoubleQuoted => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('"') => self.state = State::AfterDOCTYPEPublicIdentifier,
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.add_public_identifier(CHAR_REPLACEMENT);
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::AbruptDoctypePublicIdentifier, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => self.add_public_identifier(c.into()),
                    }
                }
                State::DOCTYPEPublicIdentifierSingleQuoted => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('\'') => self.state = State::AfterDOCTYPEPublicIdentifier,
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.add_public_identifier(CHAR_REPLACEMENT);
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::AbruptDoctypePublicIdentifier, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => self.add_public_identifier(c.into()),
                    }
                }
                State::AfterDOCTYPEPublicIdentifier => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            self.state = State::BetweenDOCTYPEPublicAndSystemIdentifiers;
                        }
                        Ch('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Ch('"') => {
                            self.parse_error_at(
                                ParserError::MissingWhitespaceBetweenDoctypePublicAndSystemIdentifiers,
                                pos,
                            );
                            self.set_system_identifier(String::new());
                            self.state = State::DOCTYPESystemIdentifierDoubleQuoted;
                        }
                        Ch('\'') => {
                            self.parse_error_at(
                                ParserError::MissingWhitespaceBetweenDoctypePublicAndSystemIdentifiers,
                                pos,
                            );
                            self.set_system_identifier(String::new());
                            self.state = State::DOCTYPESystemIdentifierSingleQuoted;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error_at(ParserError::MissingQuoteBeforeDoctypeSystemIdentifier, pos);
                            self.stream_prev();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDOCTYPE;
                        }
                    }
                }
                State::BetweenDOCTYPEPublicAndSystemIdentifiers => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            // ignore
                        }
                        Ch('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Ch('"') => {
                            self.set_system_identifier(String::new());
                            self.state = State::DOCTYPESystemIdentifierDoubleQuoted;
                        }
                        Ch('\'') => {
                            self.set_system_identifier(String::new());
                            self.state = State::DOCTYPESystemIdentifierSingleQuoted;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error_at(ParserError::MissingQuoteBeforeDoctypeSystemIdentifier, pos);
                            self.stream_prev();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDOCTYPE;
                        }
                    }
                }
                State::AfterDOCTYPESystemKeyword => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            self.state = State::BeforeDOCTYPESystemIdentifier;
                        }
                        Ch('"') => {
                            self.parse_error_at(ParserError::MissingWhitespaceAfterDoctypeSystemKeyword, pos);
                            self.set_system_identifier(String::new());
                            self.state = State::DOCTYPESystemIdentifierDoubleQuoted;
                        }
                        Ch('\'') => {
                            self.parse_error_at(ParserError::MissingWhitespaceAfterDoctypeSystemKeyword, pos);
                            self.set_system_identifier(String::new());
                            self.state = State::DOCTYPESystemIdentifierSingleQuoted;
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::MissingDoctypeSystemIdentifier, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error_at(ParserError::MissingQuoteBeforeDoctypeSystemIdentifier, pos);
                            self.stream_prev();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDOCTYPE;
                        }
                    }
                }
                State::BeforeDOCTYPESystemIdentifier => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            // ignore
                        }
                        Ch('"') => {
                            self.set_system_identifier(String::new());
                            self.state = State::DOCTYPESystemIdentifierDoubleQuoted;
                        }
                        Ch('\'') => {
                            self.set_system_identifier(String::new());
                            self.state = State::DOCTYPESystemIdentifierSingleQuoted;
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::MissingDoctypeSystemIdentifier, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error_at(ParserError::MissingQuoteBeforeDoctypeSystemIdentifier, pos);
                            self.stream_prev();
                            self.set_quirks_mode(true);
                            self.state = State::BogusDOCTYPE;
                        }
                    }
                }
                State::DOCTYPESystemIdentifierDoubleQuoted => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('"') => self.state = State::AfterDOCTYPESystemIdentifier,
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.add_system_identifier(CHAR_REPLACEMENT);
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::AbruptDoctypeSystemIdentifier, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => self.add_system_identifier(c.into()),
                    }
                }
                State::DOCTYPESystemIdentifierSingleQuoted => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('\'') => self.state = State::AfterDOCTYPESystemIdentifier,
                        Ch(CHAR_NUL) => {
                            self.parse_error_at(ParserError::UnexpectedNullCharacter, pos);
                            self.add_system_identifier(CHAR_REPLACEMENT);
                        }
                        Ch('>') => {
                            self.parse_error_at(ParserError::AbruptDoctypeSystemIdentifier, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => self.add_system_identifier(c.into()),
                    }
                }
                State::AfterDOCTYPESystemIdentifier => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(CHAR_TAB | CHAR_LF | CHAR_FF | CHAR_SPACE) => {
                            // ignore
                        }
                        Ch('>') => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInDoctype, pos);
                            self.set_quirks_mode(true);
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        _ => {
                            self.parse_error_at(ParserError::UnexpectedCharacterAfterDoctypeSystemIdentifier, pos);
                            self.stream_prev();
                            self.state = State::BogusDOCTYPE;
                        }
                    }
                }
                State::BogusDOCTYPE => {
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch('>') | StreamEnd => {
                            self.emit_current_token();
                            self.state = State::Data;
                        }
                        Ch(CHAR_NUL) => self.parse_error_at(ParserError::UnexpectedNullCharacter, pos),
                        _ => {
                            // ignore
                        }
                    }
                }
                State::CDATASection => {
                    let run = self.stream.scan_ascii_while(|b| is_plain_ascii(b) && b != b']');
                    self.consumed.push_str(run);
                    let pos = self.stream.tell_bytes();
                    let c = self.read_char();
                    match c {
                        Ch(']') => {
                            self.state = State::CDATASectionBracket;
                        }
                        StreamEnd => {
                            self.parse_error_at(ParserError::EofInCdata, pos);
                            self.state = State::Data;
                        }
                        _ => self.consume(c.into()),
                    }
                }
                State::CDATASectionBracket => {
                    let c = self.read_char();
                    if let Ch(']') = c {
                        self.state = State::CDATASectionEnd;
                    } else {
                        self.consume(']');
                        self.stream_prev();
                        self.state = State::CDATASection;
                    }
                }
                State::CDATASectionEnd => {
                    let c = self.read_char();
                    match c {
                        Ch('>') => self.state = State::Data,
                        Ch(']') => self.consume(']'),
                        StreamEnd => {
                            self.consume(']');
                            self.consume(']');
                            self.state = State::CDATASection;
                        }
                        _ => {
                            self.consume(']');
                            self.consume(']');
                            self.consume(c.into());
                            self.state = State::CDATASection;
                        }
                    }
                }
            }
        }
    }

    /// This macro reads a character from the input stream and optionally generates (tokenization)
    /// errors if the character is not valid.
    fn read_char(&mut self) -> Character {
        let pos = self.stream.tell_bytes();
        let mut c = self.stream_read_and_next();

        match c {
            Character::Surrogate(..) => {
                self.parse_error_at(ParserError::SurrogateInInputStream, pos);
                c = Ch(CHAR_REPLACEMENT);
            }
            Ch(c) if self.is_control_char(c as u32) => {
                self.parse_error_at(ParserError::ControlCharacterInInputStream, pos);
            }
            Ch(c) if self.is_noncharacter(c as u32) => {
                self.parse_error_at(ParserError::NoncharacterInInputStream, pos);
            }
            _ => {}
        }

        // println!("stream_read(): {:?}", c);

        c
    }

    /// Adds the given character to the current token's value (if applicable)
    fn add_to_token_value(&mut self, c: char) {
        if let Some(Token::Comment { comment: value, .. }) = &mut self.current_token {
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
            Some(Token::StartTag { name, .. } | Token::EndTag { name, .. }) => {
                name.push(c);
            }
            Some(Token::DocType { name, .. }) => {
                // DOCTYPE can have an optional name
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
            self.last_start_token.clear();
            self.last_start_token.push_str(name);
        }

        // If there is any consumed data, emit this first as a text token
        if self.has_consumed_data() {
            self.token_queue.push_back(Token::Text {
                text: std::mem::take(&mut self.consumed),
                location: self.stream.location_at(self.consumed_pos),
            });
        }

        self.token_queue.push_back(token);
    }

    // Consumes the given character
    pub(crate) fn consume(&mut self, c: char) {
        // Add c to the current token data
        self.consumed.push(c);
    }

    /// Pushes an end-tag and changes to the given state
    fn transition_to(&mut self, state: State) {
        self.consumed.push_str("</");
        self.consumed.push_str(&self.temporary_buffer);
        self.temporary_buffer.clear();

        self.state = state;
    }

    /// Return true when the given `end_token` matches the stored start token (ie: 'table' matches when
    /// `last_start_token` = 'table')
    fn is_appropriate_end_token(&self, end_token: &str) -> bool {
        self.last_start_token == end_token
    }

    /// Returns true when the next characters in the stream match the (ASCII) `keyword`, compared
    /// ASCII case-insensitively. The stream position is not changed.
    fn lookahead_eq_ignore_ascii_case(&mut self, keyword: &str) -> bool {
        self.stream
            .get_slice(keyword.len())
            .iter()
            .zip(keyword.chars())
            .all(|(c, k)| matches!(c, Ch(c) if c.eq_ignore_ascii_case(&k)))
    }

    /// Return the consumed string as a String
    #[must_use]
    pub fn get_consumed_str(&self) -> &str {
        &self.consumed
    }

    /// Returns true if there is anything in the consume buffer
    #[must_use]
    pub fn has_consumed_data(&self) -> bool {
        !self.consumed.is_empty()
    }

    /// Creates a parser log error message
    pub(crate) fn parse_error(&mut self, message: ParserError, location: Location) {
        self.error_logger.borrow_mut().add_error(location, message.as_str());
    }

    /// Creates a parser log error message at byte offset `pos`. The location is only worked
    /// out here, so states can note the offset of each character without paying for it.
    pub(crate) fn parse_error_at(&mut self, message: ParserError, pos: usize) {
        let location = self.stream.location_at(pos);
        self.parse_error(message, location);
    }

    /// Set `is_closing_tag` in current token
    fn set_is_closing_in_current_token(&mut self, is_closing: bool) {
        let Some(token) = self.current_token.as_mut() else {
            return;
        };
        match token {
            Token::EndTag { .. } => {
                self.stream_prev();
                self.parse_error(ParserError::EndTagWithTrailingSolidus, self.get_location());
                self.stream_next_n(1);
            }
            Token::StartTag { is_self_closing, .. } => {
                *is_self_closing = is_closing;
            }
            _ => {}
        }
    }

    /// Set `force_quirk` mode in current token
    fn set_quirks_mode(&mut self, quirky: bool) {
        if let Some(Token::DocType { force_quirks, .. }) = self.current_token.as_mut() {
            *force_quirks = quirky;
        }
    }

    /// This function checks to see if there is already an attribute name like the one in `current_attr_name`.
    fn attr_already_exists(&mut self) -> bool {
        self.current_attrs.contains_key(&self.current_attr_name)
    }

    /// Saves the current attribute name and value onto the `current_attrs` stack, if there is anything to store
    fn store_and_clear_current_attribute(&mut self) {
        if !self.current_attr_name.is_empty() && !self.current_attrs.contains_key(&self.current_attr_name) {
            self.current_attrs.insert(
                std::mem::take(&mut self.current_attr_name),
                std::mem::take(&mut self.current_attr_value),
            );
        } else {
            self.current_attr_name.clear();
            self.current_attr_value.clear();
        }
    }

    /// Drops any attribute that is still being collected. Used when the tag is dropped at EOF.
    fn clear_attribute_state(&mut self) {
        self.current_attr_name.clear();
        self.current_attr_value.clear();
        self.current_attrs.clear();
    }

    /// This method will add current generated attributes to the current (start) token if needed.
    fn add_stored_attributes_to_current_token(&mut self) {
        if self.current_attrs.is_empty() {
            return;
        }
        let Some(token) = self.current_token.as_mut() else {
            return;
        };

        match token {
            Token::EndTag { .. } => {
                // Error is one char before this one. Unread, fetch location and read again
                self.stream_prev();
                self.parse_error(ParserError::EndTagWithAttributes, self.get_location());
                self.stream_next_n(1);
                // An end tag never carries attributes, so drop them instead of leaving them
                // for the next start tag.
                self.current_attrs.clear();
            }
            Token::StartTag { attributes, .. } if attributes.is_empty() => {
                *attributes = std::mem::take(&mut self.current_attrs);
            }
            Token::StartTag { attributes, .. } => {
                attributes.extend(std::mem::take(&mut self.current_attrs));
            }
            _ => {}
        }
    }

    fn stream_read_and_next(&mut self) -> Character {
        let c = self.stream.read_and_next();
        self.last_char = c;
        c
    }

    fn stream_prev(&mut self) {
        if self.last_char == StreamEnd {
            return;
        }

        self.stream.prev();
    }

    fn stream_next_n(&mut self, n: usize) {
        for _ in 0..n {
            self.stream_read_and_next();
        }
    }
}

#[cfg(test)]
mod location_tests {
    use super::*;
    use crate::node::SVG_NAMESPACE;
    use gosub_shared::byte_stream::Encoding;

    /// Tokenizes `html` and returns `(text, line, column)` for every text and comment token.
    /// Comments are wrapped in `<!--` and `-->` so they can be told apart from text.
    fn located(html: &str, opts: Option<Options>, namespace: &'static str) -> Vec<(String, usize, usize)> {
        let mut stream = ByteStream::from_str(html, Encoding::UTF8);
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
        let mut tokenizer = Tokenizer::new(&mut stream, opts, error_logger);

        let mut result = vec![];
        loop {
            let parser_data = ParserData {
                adjusted_node_namespace: namespace.into(),
            };
            match tokenizer.next_token(parser_data).unwrap() {
                Token::Text { text, location } => result.push((text, location.line, location.column)),
                Token::Comment { comment, location } => {
                    result.push((format!("<!--{comment}-->"), location.line, location.column));
                }
                Token::Eof { .. } => break,
                _ => {}
            }
        }
        result
    }

    fn html(html: &str) -> Vec<(String, usize, usize)> {
        located(html, None, HTML_NAMESPACE)
    }

    fn in_state(html: &str, state: State, last_start_tag: &str) -> Vec<(String, usize, usize)> {
        let opts = Options {
            initial_state: state,
            last_start_tag: last_start_tag.to_string(),
        };
        located(html, Some(opts), HTML_NAMESPACE)
    }

    fn t(text: &str, line: usize, column: usize) -> (String, usize, usize) {
        (text.to_string(), line, column)
    }

    #[test]
    fn text_at_document_start() {
        assert_eq!(html("hello"), vec![t("hello", 1, 1)]);
    }

    #[test]
    fn text_before_start_tag() {
        assert_eq!(html("<p>hello<b>x"), vec![t("hello", 1, 4), t("x", 1, 12)]);
    }

    #[test]
    fn text_before_end_tag() {
        assert_eq!(html("<p>ab</p>"), vec![t("ab", 1, 4)]);
    }

    #[test]
    fn text_before_eof() {
        assert_eq!(html("<p>tail"), vec![t("tail", 1, 4)]);
    }

    #[test]
    fn text_over_several_lines() {
        assert_eq!(html("<p>\n<b>\n  x</b>"), vec![t("\n", 1, 4), t("\n  x", 2, 4)]);
    }

    #[test]
    fn text_after_crlf() {
        assert_eq!(html("<p>a\r\nb<b>\r\nc"), vec![t("a\nb", 1, 4), t("\nc", 2, 5)]);
    }

    #[test]
    fn text_with_character_references() {
        assert_eq!(html("<p>&amp;x<b>y"), vec![t("&x", 1, 4), t("y", 1, 13)]);
        assert_eq!(html("<p>a&lt;b"), vec![t("a<b", 1, 4)]);
    }

    #[test]
    fn text_with_null() {
        assert_eq!(html("<p>\0x"), vec![t("\0x", 1, 4)]);
    }

    #[test]
    fn text_that_starts_with_a_less_than_sign() {
        assert_eq!(html("<p>< x<b>"), vec![t("< x", 1, 4)]);
        assert_eq!(html("a<b>< x"), vec![t("a", 1, 1), t("< x", 1, 5)]);
        assert_eq!(html("<p><"), vec![t("<", 1, 4)]);
        assert_eq!(html("<p></"), vec![t("</", 1, 4)]);
    }

    #[test]
    fn text_after_dropped_end_tag() {
        assert_eq!(html("<p></>b"), vec![t("b", 1, 7)]);
    }

    #[test]
    fn text_after_comment() {
        assert_eq!(html("<!--a-->b"), vec![t("<!--a-->", 1, 1), t("b", 1, 9)]);
    }

    #[test]
    fn text_in_rcdata_rawtext_and_script_data() {
        assert_eq!(
            in_state("a&amp;b</title>c", State::RCDATA, "title"),
            vec![t("a&b", 1, 1), t("c", 1, 16)]
        );
        assert_eq!(
            in_state("x</y>z</style>w", State::RAWTEXT, "style"),
            vec![t("x</y>z", 1, 1), t("w", 1, 15)]
        );
        assert_eq!(
            in_state("x</y>z</script>w", State::ScriptData, "script"),
            vec![t("x</y>z", 1, 1), t("w", 1, 16)]
        );
        assert_eq!(
            in_state("<!--<script>a</script>-->b</script>c", State::ScriptData, "script"),
            vec![t("<!--<script>a</script>-->b", 1, 1), t("c", 1, 36)]
        );
    }

    #[test]
    fn text_in_cdata_section() {
        assert_eq!(located("<svg><![CDATA[x]]>", None, SVG_NAMESPACE), vec![t("x", 1, 15)]);
        assert_eq!(located("<svg>a<![CDATA[x]]>", None, SVG_NAMESPACE), vec![t("ax", 1, 6)]);
    }

    #[test]
    fn comment_starts_at_its_less_than_sign() {
        assert_eq!(html("<p><!--x-->"), vec![t("<!--x-->", 1, 4)]);
        assert_eq!(html("<p><!---->"), vec![t("<!---->", 1, 4)]);
        assert_eq!(html("<p><!-->"), vec![t("<!---->", 1, 4)]);
        assert_eq!(html("<p>\n  <!--x"), vec![t("\n  ", 1, 4), t("<!--x-->", 2, 3)]);
    }

    /// Tokenizes `html` and returns every token (without location) and every error as
    /// `message@line:column`.
    fn tokens_and_errors(html: &str, state: State) -> (Vec<String>, Vec<String>) {
        let mut stream = ByteStream::from_str(html, Encoding::UTF8);
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
        let opts = Options {
            initial_state: state,
            last_start_tag: String::new(),
        };
        let mut tokenizer = Tokenizer::new(&mut stream, Some(opts), error_logger.clone());
        let mut tokens = vec![];
        loop {
            let token = tokenizer.next_token(ParserData::default()).unwrap();
            if token.is_eof() {
                break;
            }
            tokens.push(format!("{token:?}").split(", location").next().unwrap().to_string());
        }
        let errors = error_logger
            .borrow()
            .get_errors()
            .iter()
            .map(|e| format!("{}@{}:{}", e.message, e.location.line, e.location.column))
            .collect();
        (tokens, errors)
    }

    #[test]
    fn bulk_scans_stop_at_characters_that_need_attention() {
        // Control characters, NUL and non-ASCII break a run and are still reported
        let (tokens, errors) = tokens_and_errors("ab\x01c\0d\u{e9}\u{fffe}e\r\nf\x7f", State::Data);
        assert_eq!(
            tokens,
            vec!["Text { text: \"ab\\u{1}c\\0d\u{e9}\\u{fffe}e\\nf\\u{7f}\""]
        );
        assert_eq!(
            errors,
            vec![
                "control-character-in-input-stream@1:3",
                "unexpected-null-character@1:5",
                "noncharacter-in-input-stream@1:8",
                "control-character-in-input-stream@2:2",
            ]
        );

        // Same in the other text states, with their own replacement of NUL
        for state in [State::RCDATA, State::RAWTEXT, State::ScriptData, State::PLAINTEXT] {
            let (tokens, errors) = tokens_and_errors("ab\x01c\0d", state);
            assert_eq!(tokens, vec!["Text { text: \"ab\\u{1}c\u{fffd}d\""], "{state:?}");
            assert_eq!(
                errors,
                vec!["control-character-in-input-stream@1:3", "unexpected-null-character@1:5"],
                "{state:?}"
            );
        }
    }

    #[test]
    fn bulk_scans_in_tags_and_comments() {
        let (tokens, errors) = tokens_and_errors(
            "<DiV\x01x CLASS=\"a&amp;b\x01\" Data-X='c&lt;d' u=e&gt;f\x01g\u{e9}h>",
            State::Data,
        );
        assert_eq!(
            tokens,
            vec![
                "StartTag { name: \"div\\u{1}x\", is_self_closing: false, attributes: Attributes { entries: \
                 {\"class\": \"a&b\\u{1}\", \"data-x\": \"c<d\", \"u\": \"e>f\\u{1}g\u{e9}h\"} }"
            ]
        );
        assert_eq!(
            errors,
            vec![
                "control-character-in-input-stream@1:5",
                "control-character-in-input-stream@1:22",
                "control-character-in-input-stream@1:49",
            ]
        );

        // Text and tag name runs stop at the '<', '/' and whitespace that end them
        let (tokens, _) = tokens_and_errors("x<a/><B\tc>y</b >", State::Data);
        assert_eq!(
            tokens,
            vec![
                r#"Text { text: "x""#,
                r#"StartTag { name: "a", is_self_closing: true, attributes: Attributes { entries: {} }"#,
                r#"StartTag { name: "b", is_self_closing: false, attributes: Attributes { entries: {"c": ""} }"#,
                r#"Text { text: "y""#,
                r#"EndTag { name: "b""#,
            ]
        );

        let (tokens, errors) = tokens_and_errors("<!--a<b-c\x01--><!x\0y>", State::Data);
        assert_eq!(
            tokens,
            vec![
                r#"Comment { comment: "a<b-c\u{1}""#,
                "Comment { comment: \"x\u{fffd}y\""
            ]
        );
        assert_eq!(
            errors,
            vec![
                "control-character-in-input-stream@1:10",
                "incorrectly-opened-comment@1:16",
                "unexpected-null-character@1:17",
            ]
        );
    }

    #[test]
    fn tokens_inserted_at_queue_start_come_first_in_order() {
        let mut stream = ByteStream::from_str("<c><d>", Encoding::UTF8);
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
        let mut tokenizer = Tokenizer::new(&mut stream, None, error_logger);
        let tag = |name: &str| Token::StartTag {
            name: name.to_string(),
            is_self_closing: false,
            attributes: Attributes::new(),
            location: Location::default(),
        };

        let mut names = vec![];
        let mut next = |tokenizer: &mut Tokenizer| {
            let mut token = tokenizer.next_token(ParserData::default()).unwrap();
            token.set_location(Location::default());
            names.push(token.to_string());
        };

        next(&mut tokenizer);
        tokenizer.insert_tokens_at_queue_start(vec![tag("a"), tag("b")]);
        next(&mut tokenizer);
        tokenizer.insert_tokens_at_queue_start(vec![tag("x")]);
        next(&mut tokenizer);
        next(&mut tokenizer);
        next(&mut tokenizer);
        next(&mut tokenizer);

        assert_eq!(names, vec!["<c>", "<a>", "<x>", "<b>", "<d>", "EOF"]);
    }

    #[test]
    fn bogus_comment_starts_at_its_less_than_sign() {
        assert_eq!(html("<p><!x>"), vec![t("<!--x-->", 1, 4)]);
        assert_eq!(html("<p><?x>"), vec![t("<!--?x-->", 1, 4)]);
        assert_eq!(html("<p></ x>"), vec![t("<!-- x-->", 1, 4)]);
        assert_eq!(html("<p><![CDATA[x]]>"), vec![t("<!--[CDATA[x]]-->", 1, 4)]);
    }
}
