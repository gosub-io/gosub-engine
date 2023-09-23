use crate::html5_parser::error_logger::ParserError;
use crate::html5_parser::input_stream::Element;
use crate::read_char;

extern crate lazy_static;
use crate::html5_parser::input_stream::SeekMode::SeekCur;
use crate::html5_parser::tokenizer::replacement_tables::{TOKEN_NAMED_CHARS, TOKEN_REPLACEMENTS};
use crate::html5_parser::tokenizer::{Tokenizer, CHAR_REPLACEMENT};
use lazy_static::lazy_static;

// Different states for the character references
pub enum CcrState {
    CharacterReference,
    NamedCharacterReference,
    AmbiguousAmpersand,
    NumericCharacterReference,
    HexadecimalCharacterReferenceStart,
    DecimalCharacterReferenceStart,
    HexadecimalCharacterReference,
    DecimalCharacterReference,
    NumericalCharacterReferenceEnd,
}

macro_rules! consume_temp_buffer {
    ($self:expr, $as_attribute:expr) => {
        for c in $self.temporary_buffer.clone() {
            if $as_attribute {
                $self.current_attr_value.push(c);
            } else {
                $self.consume(c);
            }
        }
        $self.temporary_buffer.clear();
    };
}

impl<'a> Tokenizer<'a> {
    // Consumes a character reference and places this in the tokenizer consume buffer
    // ref: 8.2.4.69 Tokenizing character references

    // @TODO: fix additional allowed char
    pub fn consume_character_reference(
        &mut self,
        _additional_allowed_char: Option<Element>,
        as_attribute: bool,
    ) {
        let mut ccr_state = CcrState::CharacterReference;
        let mut char_ref_code: Option<u32> = Some(0);

        loop {
            match ccr_state {
                CcrState::CharacterReference => {
                    self.temporary_buffer = vec!['&'];

                    let c = read_char!(self);
                    match c {
                        // Element::Eof => {
                        //     consume_temp_buffer!(self, as_attribute);
                        //     return
                        // },
                        Element::Utf8('A'..='Z')
                        | Element::Utf8('a'..='z')
                        | Element::Utf8('0'..='9') => {
                            self.stream.unread();
                            ccr_state = CcrState::NamedCharacterReference;
                        }
                        Element::Utf8('#') => {
                            self.temporary_buffer.push(c.utf8());
                            ccr_state = CcrState::NumericCharacterReference;
                        }
                        _ => {
                            consume_temp_buffer!(self, as_attribute);

                            self.stream.unread();
                            return;
                        }
                    }
                }
                CcrState::NamedCharacterReference => {
                    if let Some(entity) = self.find_entity() {
                        self.stream.seek(SeekCur, entity.len() as isize);
                        let c = self.stream.look_ahead(0);
                        if as_attribute
                            && !entity.ends_with(';')
                            && c.is_utf8()
                            && (c.utf8() == '=' || c.utf8().is_ascii_alphanumeric())
                        {
                            // for historical reasons, the codepoints should be flushed as is
                            for c in entity.chars() {
                                self.temporary_buffer.push(c);
                            }

                            consume_temp_buffer!(self, as_attribute);
                            return;
                        }

                        let entity_chars = *TOKEN_NAMED_CHARS.get(entity.as_str()).unwrap();

                        // Flush codepoints consumed as character reference
                        for c in entity_chars.chars() {
                            if as_attribute {
                                self.current_attr_value.push(c);
                            } else {
                                self.consume(c);
                            }
                        }
                        self.temporary_buffer.clear();

                        if !entity.ends_with(';') {
                            // We need to return the position where we expected the ';'
                            self.stream.read_char(); // We can't use skip, as this might interfere with EOF stuff (fix it)
                            self.parse_error(ParserError::MissingSemicolonAfterCharacterReference);
                            self.stream.unread();
                        }

                        return;
                    }

                    consume_temp_buffer!(self, as_attribute);
                    ccr_state = CcrState::AmbiguousAmpersand;
                }
                CcrState::AmbiguousAmpersand => {
                    let c = read_char!(self);
                    match c {
                        // Element::Eof => return,
                        Element::Utf8('A'..='Z')
                        | Element::Utf8('a'..='z')
                        | Element::Utf8('0'..='9') => {
                            if as_attribute {
                                self.current_attr_value.push(c.utf8());
                            } else {
                                self.consume(c.utf8());
                            }
                        }
                        Element::Utf8(';') => {
                            self.parse_error(ParserError::UnknownNamedCharacterReference);
                            self.stream.unread();
                            return;
                        }
                        _ => {
                            self.stream.unread();
                            return;
                        }
                    }
                }
                CcrState::NumericCharacterReference => {
                    char_ref_code = Some(0);

                    let c = read_char!(self);
                    match c {
                        // Element::Eof => ccr_state = CcrState::NumericalCharacterReferenceEndState,
                        Element::Utf8('X') | Element::Utf8('x') => {
                            self.temporary_buffer.push(c.utf8());
                            ccr_state = CcrState::HexadecimalCharacterReferenceStart;
                        }
                        _ => {
                            self.stream.unread();
                            ccr_state = CcrState::DecimalCharacterReferenceStart;
                        }
                    }
                }
                CcrState::HexadecimalCharacterReferenceStart => {
                    let c = read_char!(self);
                    match c {
                        // Element::Eof => ccr_state = CcrState::NumericalCharacterReferenceEndState,
                        Element::Utf8('0'..='9')
                        | Element::Utf8('A'..='F')
                        | Element::Utf8('a'..='f') => {
                            self.stream.unread();
                            ccr_state = CcrState::HexadecimalCharacterReference
                        }
                        _ => {
                            self.parse_error(
                                ParserError::AbsenceOfDigitsInNumericCharacterReference,
                            );
                            consume_temp_buffer!(self, as_attribute);

                            self.stream.unread();
                            return;
                        }
                    }
                }
                CcrState::DecimalCharacterReferenceStart => {
                    let c = read_char!(self);
                    match c {
                        Element::Utf8('0'..='9') => {
                            self.stream.unread();
                            ccr_state = CcrState::DecimalCharacterReference;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::AbsenceOfDigitsInNumericCharacterReference,
                            );
                            consume_temp_buffer!(self, as_attribute);

                            self.stream.unread();
                            return;
                        }
                    }
                }
                CcrState::HexadecimalCharacterReference => {
                    let c = read_char!(self);
                    match c {
                        // Element::Eof => ccr_state = CcrState::NumericalCharacterReferenceEndState,
                        Element::Utf8('0'..='9') => {
                            let i = c.utf8() as u32 - 0x30;
                            if let Some(value) = char_ref_code {
                                char_ref_code = value
                                    .checked_mul(16)
                                    .and_then(|mul_result| mul_result.checked_add(i));
                            }
                        }
                        Element::Utf8('A'..='F') => {
                            let i = c.utf8() as u32 - 0x37;
                            if let Some(value) = char_ref_code {
                                char_ref_code = value
                                    .checked_mul(16)
                                    .and_then(|mul_result| mul_result.checked_add(i));
                            }
                        }
                        Element::Utf8('a'..='f') => {
                            let i = c.utf8() as u32 - 0x57;
                            if let Some(value) = char_ref_code {
                                char_ref_code = value
                                    .checked_mul(16)
                                    .and_then(|mul_result| mul_result.checked_add(i));
                            }
                        }
                        Element::Utf8(';') => {
                            ccr_state = CcrState::NumericalCharacterReferenceEnd;
                        }
                        _ => {
                            self.parse_error(ParserError::MissingSemicolonAfterCharacterReference);
                            self.stream.unread();
                            ccr_state = CcrState::NumericalCharacterReferenceEnd;
                        }
                    }
                }
                CcrState::DecimalCharacterReference => {
                    let c = read_char!(self);
                    match c {
                        // Element::Eof => ccr_state = CcrState::NumericalCharacterReferenceEndState,
                        Element::Utf8('0'..='9') => {
                            let i = c.utf8() as u32 - 0x30;
                            if let Some(value) = char_ref_code {
                                char_ref_code = value
                                    .checked_mul(10)
                                    .and_then(|mul_result| mul_result.checked_add(i));
                            }
                        }
                        Element::Utf8(';') => {
                            ccr_state = CcrState::NumericalCharacterReferenceEnd;
                        }
                        _ => {
                            self.parse_error(ParserError::MissingSemicolonAfterCharacterReference);
                            self.stream.unread();
                            ccr_state = CcrState::NumericalCharacterReferenceEnd;
                        }
                    }
                }
                CcrState::NumericalCharacterReferenceEnd => {
                    let overflow = char_ref_code.is_none();
                    let mut char_ref_code = char_ref_code.unwrap_or(0);

                    if char_ref_code == 0 && !overflow {
                        self.stream.read_char();
                        self.parse_error(ParserError::NullCharacterReference);
                        char_ref_code = CHAR_REPLACEMENT as u32;
                    }

                    if char_ref_code > 0x10FFFF || overflow {
                        self.stream.read_char();
                        self.parse_error(ParserError::CharacterReferenceOutsideUnicodeRange);
                        self.stream.unread();
                        char_ref_code = CHAR_REPLACEMENT as u32;
                    }

                    if self.is_surrogate(char_ref_code) {
                        self.stream.read_char();
                        self.parse_error(ParserError::SurrogateCharacterReference);
                        self.stream.unread();
                        char_ref_code = CHAR_REPLACEMENT as u32;
                    }
                    if self.is_noncharacter(char_ref_code) {
                        self.stream.read_char();
                        self.parse_error(ParserError::NoncharacterCharacterReference);
                        self.stream.unread();
                        // char_ref_code = CHAR_REPLACEMENT as u32;
                    }
                    if self.is_control_char(char_ref_code) || char_ref_code == 0x0D {
                        self.stream.read_char();
                        self.stream.read_char();
                        self.parse_error(ParserError::ControlCharacterReference);
                        // self.stream.unread();
                        self.stream.unread();

                        if TOKEN_REPLACEMENTS.contains_key(&char_ref_code) {
                            char_ref_code = *TOKEN_REPLACEMENTS.get(&char_ref_code).unwrap() as u32;
                        }
                    }

                    self.temporary_buffer =
                        vec![char::from_u32(char_ref_code).unwrap_or(CHAR_REPLACEMENT)];
                    consume_temp_buffer!(self, as_attribute);

                    return;
                }
            }
        }
    }

    pub(crate) fn is_surrogate(&self, num: u32) -> bool {
        (0xD800..=0xDFFF).contains(&num)
    }

    pub(crate) fn is_noncharacter(&self, num: u32) -> bool {
        (0xFDD0..=0xFDEF).contains(&num)
            || [
                0xFFFE, 0xFFFF, 0x1FFFE, 0x1FFFF, 0x2FFFE, 0x2FFFF, 0x3FFFE, 0x3FFFF, 0x4FFFE,
                0x4FFFF, 0x5FFFE, 0x5FFFF, 0x6FFFE, 0x6FFFF, 0x7FFFE, 0x7FFFF, 0x8FFFE, 0x8FFFF,
                0x9FFFE, 0x9FFFF, 0xAFFFE, 0xAFFFF, 0xBFFFE, 0xBFFFF, 0xCFFFE, 0xCFFFF, 0xDFFFE,
                0xDFFFF, 0xEFFFE, 0xEFFFF, 0xFFFFE, 0xFFFFF, 0x10FFFE, 0x10FFFF,
            ]
            .contains(&num)
    }

    pub(crate) fn is_control_char(&self, num: u32) -> bool {
        // White spaces are ok
        if [0x0009, 0x000A, 0x000C, 0x000D, 0x0020].contains(&num) {
            return false;
        }

        (0x0001..=0x001F).contains(&num) || (0x007F..=0x009F).contains(&num)
    }

    // Finds the longest entity from the current position in the stream. Returns the entity
    // replacement OR None when no entity has been found.
    fn find_entity(&mut self) -> Option<String> {
        let s = self.stream.look_ahead_slice(*LONGEST_ENTITY_LENGTH);
        for i in (0..=s.len()).rev() {
            if TOKEN_NAMED_CHARS.contains_key(&s[0..i]) {
                // Move forward with the number of chars matching
                // self.stream.skip(i);
                return Some(String::from(&s[0..i]));
            }
        }
        None
    }
}

lazy_static! {
    // Returns the longest entity in the TOKEN_NAMED_CHARS map (this could be a const actually)
    static ref LONGEST_ENTITY_LENGTH: usize = {
        TOKEN_NAMED_CHARS.keys().map(|key| key.len()).max().unwrap_or(0)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html5_parser::error_logger::ErrorLogger;
    use crate::html5_parser::input_stream::InputStream;
    use std::cell::RefCell;
    use std::rc::Rc;

    macro_rules! entity_tests {
        ($($name:ident : $value:expr)*) => {
            $(
                #[test]
                fn $name() {
                    let (input, expected) = $value;

                    let mut is = InputStream::new();
                    is.read_from_str(input, None);

                    let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
                    let mut tokenizer = Tokenizer::new(&mut is, None, error_logger.clone());

                    let token = tokenizer.next_token();
                    assert_eq!(expected, token.to_string());
                }
            )*
        }
    }
}
