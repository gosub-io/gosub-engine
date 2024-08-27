extern crate lazy_static;

use crate::error_logger::ParserError;
use crate::tokenizer::replacement_tables::{TOKEN_NAMED_CHARS, TOKEN_REPLACEMENTS};
use crate::tokenizer::{Tokenizer, CHAR_REPLACEMENT};
use gosub_shared::byte_stream::Character::Ch;
use gosub_shared::byte_stream::{Character, Stream};
use lazy_static::lazy_static;

/// Different states for the character references
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

impl Tokenizer<'_> {
    /// Consumes a character reference and places this in the tokenizer consume buffer
    /// ref: 8.2.4.69 Tokenizing character references
    ///
    /// @TODO: fix additional allowed char
    pub fn consume_character_reference(
        &mut self,
        _additional_allowed_char: Option<Character>,
        as_attribute: bool,
    ) {
        let mut ccr_state = CcrState::CharacterReference;
        let mut char_ref_code: Option<u32> = Some(0);

        loop {
            match ccr_state {
                CcrState::CharacterReference => {
                    self.temporary_buffer.clear();
                    self.temporary_buffer.push('&');

                    let c = self.stream_read_and_next();
                    match c {
                        Ch(ch) if ch.is_ascii_alphanumeric() => {
                            self.stream_prev();
                            ccr_state = CcrState::NamedCharacterReference;
                        }
                        Ch(c @ '#') => {
                            self.temporary_buffer.push(c);
                            ccr_state = CcrState::NumericCharacterReference;
                        }
                        Character::StreamEnd => {
                            self.consume_temp_buffer(as_attribute);
                            return;
                        }
                        _ => {
                            self.consume_temp_buffer(as_attribute);
                            self.stream_prev();
                            return;
                        }
                    }
                }
                CcrState::NamedCharacterReference => {
                    if let Some(entity) = self.find_entity() {
                        self.stream_next_n(entity.len());
                        let c = self.stream.look_ahead(0);

                        if as_attribute
                            && !entity.ends_with(';')
                            && (c == Ch('=') || matches!(c, Ch(c) if c.is_ascii_alphanumeric()))
                        {
                            // for historical reasons, the codepoints should be flushed as
                            // is
                            for c in entity.chars() {
                                self.temporary_buffer.push(c);
                            }

                            self.consume_temp_buffer(as_attribute);
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
                            self.parse_error(
                                ParserError::MissingSemicolonAfterCharacterReference,
                                self.get_location(),
                            );
                        }

                        return;
                    }

                    self.consume_temp_buffer(as_attribute);
                    ccr_state = CcrState::AmbiguousAmpersand;
                }
                CcrState::AmbiguousAmpersand => {
                    let c = self.stream_read_and_next();
                    match c {
                        // Element::Eof => return,
                        Ch(ch) if ch.is_ascii_alphanumeric() => {
                            if as_attribute {
                                self.current_attr_value.push(c.into());
                            } else {
                                self.consume(c.into());
                            }
                        }
                        Ch(';') => {
                            self.stream_prev();
                            self.parse_error(
                                ParserError::UnknownNamedCharacterReference,
                                self.get_location(),
                            );
                            return;
                        }
                        Character::StreamEnd => {
                            // self.consume_temp_buffer(as_attribute);
                            return;
                        }
                        _ => {
                            self.stream_prev();
                            return;
                        }
                    }
                }
                CcrState::NumericCharacterReference => {
                    char_ref_code = Some(0);

                    let c = self.stream_read_and_next();
                    match c {
                        Ch('X' | 'x') => {
                            // Element::Eof => ccr_state = CcrState::NumericalCharacterReferenceEnd,
                            self.temporary_buffer.push(c.into());
                            ccr_state = CcrState::HexadecimalCharacterReferenceStart;
                        }
                        Character::StreamEnd => {
                            ccr_state = CcrState::DecimalCharacterReferenceStart;
                        }
                        _ => {
                            self.stream_prev();
                            ccr_state = CcrState::DecimalCharacterReferenceStart;
                        }
                    }
                }
                CcrState::HexadecimalCharacterReferenceStart => {
                    let loc = self.get_location();
                    let c = self.stream_read_and_next();
                    match c {
                        Ch('0'..='9' | 'A'..='F' | 'a'..='f') => {
                            // Element::Eof => ccr_state = CcrState::NumericalCharacterReferenceEnd,
                            self.stream_prev();
                            ccr_state = CcrState::HexadecimalCharacterReference;
                        }
                        Character::StreamEnd => {
                            self.parse_error(
                                ParserError::AbsenceOfDigitsInNumericCharacterReference,
                                loc,
                            );
                            self.consume_temp_buffer(as_attribute);
                            return;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::AbsenceOfDigitsInNumericCharacterReference,
                                loc,
                            );
                            self.consume_temp_buffer(as_attribute);

                            self.stream_prev();
                            return;
                        }
                    }
                }
                CcrState::DecimalCharacterReferenceStart => {
                    let loc = self.get_location();
                    let c = self.stream_read_and_next();
                    match c {
                        Ch('0'..='9') => {
                            self.stream_prev();
                            ccr_state = CcrState::DecimalCharacterReference;
                        }
                        Character::StreamEnd => {
                            self.parse_error(
                                ParserError::AbsenceOfDigitsInNumericCharacterReference,
                                loc,
                            );
                            self.consume_temp_buffer(as_attribute);
                            return;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::AbsenceOfDigitsInNumericCharacterReference,
                                loc,
                            );
                            self.consume_temp_buffer(as_attribute);

                            self.stream_prev();
                            return;
                        }
                    }
                }
                CcrState::HexadecimalCharacterReference => {
                    let loc = self.get_location();
                    let c = self.stream_read_and_next();
                    match c {
                        // Element::Eof => ccr_state = CcrState::NumericalCharacterReferenceEnd,
                        Ch(c @ '0'..='9') => {
                            let i = c as u32 - 0x30;
                            if let Some(value) = char_ref_code {
                                char_ref_code = value
                                    .checked_mul(16)
                                    .and_then(|mul_result| mul_result.checked_add(i));
                            }
                        }
                        Ch(c @ 'A'..='F') => {
                            let i = c as u32 - 0x37;
                            if let Some(value) = char_ref_code {
                                char_ref_code = value
                                    .checked_mul(16)
                                    .and_then(|mul_result| mul_result.checked_add(i));
                            }
                        }
                        Ch(c @ 'a'..='f') => {
                            let i = c as u32 - 0x57;
                            if let Some(value) = char_ref_code {
                                char_ref_code = value
                                    .checked_mul(16)
                                    .and_then(|mul_result| mul_result.checked_add(i));
                            }
                        }
                        Ch(';') => {
                            ccr_state = CcrState::NumericalCharacterReferenceEnd;
                        }
                        Character::StreamEnd => {
                            self.parse_error(
                                ParserError::MissingSemicolonAfterCharacterReference,
                                loc,
                            );
                            ccr_state = CcrState::NumericalCharacterReferenceEnd;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingSemicolonAfterCharacterReference,
                                loc,
                            );
                            self.stream_prev();
                            ccr_state = CcrState::NumericalCharacterReferenceEnd;
                        }
                    }
                }
                CcrState::DecimalCharacterReference => {
                    let loc = self.get_location();
                    let c = self.stream_read_and_next();
                    match c {
                        // Element::Eof => ccr_state = CcrState::NumericalCharacterReferenceEndState,
                        Ch(c @ '0'..='9') => {
                            let i = c as u32 - 0x30;
                            if let Some(value) = char_ref_code {
                                char_ref_code = value
                                    .checked_mul(10)
                                    .and_then(|mul_result| mul_result.checked_add(i));
                            }
                        }
                        Ch(';') => {
                            ccr_state = CcrState::NumericalCharacterReferenceEnd;
                        }
                        Character::StreamEnd => {
                            self.parse_error(
                                ParserError::MissingSemicolonAfterCharacterReference,
                                loc,
                            );
                            ccr_state = CcrState::NumericalCharacterReferenceEnd;
                        }
                        _ => {
                            self.parse_error(
                                ParserError::MissingSemicolonAfterCharacterReference,
                                loc,
                            );
                            ccr_state = CcrState::NumericalCharacterReferenceEnd;
                            self.stream_prev();
                        }
                    }
                }
                CcrState::NumericalCharacterReferenceEnd => {
                    let overflow = char_ref_code.is_none();
                    let mut char_ref_code = char_ref_code.unwrap_or(0);

                    if char_ref_code == 0 && !overflow {
                        self.parse_error(ParserError::NullCharacterReference, self.get_location());
                        char_ref_code = CHAR_REPLACEMENT as u32;
                    }

                    if char_ref_code > 0x10FFFF || overflow {
                        self.parse_error(
                            ParserError::CharacterReferenceOutsideUnicodeRange,
                            self.get_location(),
                        );
                        char_ref_code = CHAR_REPLACEMENT as u32;
                    }

                    if self.is_surrogate(char_ref_code) {
                        self.parse_error(
                            ParserError::SurrogateCharacterReference,
                            self.get_location(),
                        );
                        char_ref_code = CHAR_REPLACEMENT as u32;
                    }
                    if self.is_noncharacter(char_ref_code) {
                        self.parse_error(
                            ParserError::NoncharacterCharacterReference,
                            self.get_location(),
                        );
                        // char_ref_code = CHAR_REPLACEMENT as u32;
                    }
                    if self.is_control_char(char_ref_code) || char_ref_code == 0x0D {
                        self.parse_error(
                            ParserError::ControlCharacterReference,
                            self.get_location(),
                        );

                        if TOKEN_REPLACEMENTS.contains_key(&char_ref_code) {
                            char_ref_code = *TOKEN_REPLACEMENTS.get(&char_ref_code).unwrap() as u32;
                        }
                    }

                    self.temporary_buffer.clear();
                    let c = char::from_u32(char_ref_code).unwrap_or(CHAR_REPLACEMENT);
                    self.temporary_buffer.push(c);
                    self.consume_temp_buffer(as_attribute);

                    return;
                }
            }
        }
    }

    fn consume_temp_buffer(&mut self, as_attribute: bool) {
        if as_attribute {
            self.current_attr_value.push_str(&self.temporary_buffer);
        } else {
            self.consumed.push_str(&self.temporary_buffer);
        }
        self.temporary_buffer.clear();
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

    /// Finds the longest entity from the current position in the stream. Returns the entity
    /// replacement OR None when no entity has been found.
    fn find_entity(&mut self) -> Option<String> {
        let chars = self.stream.get_slice(*LONGEST_ENTITY_LENGTH);

        for i in (0..=chars.len()).rev() {
            if let Some(slice) = chars.get(0..i) {
                let entity: String = slice.iter().map(|c| c.to_string()).collect();
                if TOKEN_NAMED_CHARS.contains_key(entity.as_str()) {
                    return Some(entity);
                }
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
    use crate::error_logger::ErrorLogger;
    use crate::tokenizer::{ParserData, Tokenizer};
    use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
    use std::cell::RefCell;
    use std::rc::Rc;

    macro_rules! entity_tests {
        ($($name:ident : $value:expr)*) => {
            $(
                #[test]
                fn $name() {
                    let (input, expected) = $value;

                    let mut stream = ByteStream::new(Encoding::UTF8, None);
                    stream.read_from_str(input, None);
                    stream.close();

                    let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
                    let mut tokenizer = Tokenizer::new(&mut stream, None, error_logger.clone(), Location::default());

                    let token = tokenizer.next_token(ParserData::default()).unwrap();
                    assert_eq!(expected, token.to_string());
                }
            )*
        }
    }

    entity_tests! {
        // Numbers
        entity_0: ("&#10;", "\n")
        entity_1: ("&#0;", "�")
        entity_2: ("&#x0;", "�")
        entity_3: ("&#xdeadbeef;", "�")     // replace with replacement char
        entity_4: ("&#xd888;", "�")         // replace with replacement char
        entity_5: ("&#xbeef;", "뻯")
        entity_6: ("&#x10;", "\u{10}")
        entity_7: ("&#;", "&#;")
        entity_8: ("&;", "&;")
        entity_9: ("&", "&")
        entity_10: ("&#x1;", "\u{1}")                // reserved codepoint
        entity_11: ("&#x0008;", "\u{8}")             // reserved codepoint
        entity_12: ("&#0008;", "\u{8}")              // reserved codepoint
        entity_13: ("&#8;", "\u{8}")                 // reserved codepoint
        entity_14: ("&#x0009;", "\t")
        entity_15: ("&#x007F;", "\u{7f}")
        entity_16: ("&#x80;", "\u{20ac}")
        entity_17: ("&#x82;", "\u{201a}")
        entity_18: ("&#X8c;", "\u{0152}")
        entity_19: ("&#x8d;", "\u{8d}")


        // Entities
        entity_100: ("&copy;", "©")
        entity_101: ("&copyThing;", "©Thing;")
        entity_102: ("&raquo;", "»")
        entity_103: ("&laquo;", "«")
        entity_104: ("&not;", "¬")
        entity_105: ("&notit;", "¬it;")
        entity_106: ("&notin;", "∉")
        entity_107: ("&fo", "&fo")
        entity_108: ("&xxx", "&xxx")
        entity_109: ("&copy", "©")
        entity_110: ("&copy ", "© ")
        entity_111: ("&copya", "©a")
        entity_112: ("&copya;", "©a;")
        entity_113: ("&#169;", "©")
        entity_114: ("&copy&", "©&")
        entity_115: ("&copya ", "©a ")
        entity_116: ("&#169X ", "©X ")

        // // ChatGPT generated tests
        entity_200: ("&copy;", "©")
        entity_201: ("&copy ", "© ")
        entity_202: ("&#169;", "©")
        entity_203: ("&#xA9;", "©")
        entity_204: ("&lt;", "<")
        entity_205: ("&unknown;", "&unknown;")
        entity_206: ("&#60;", "<")
        entity_207: ("&#x3C;", "<")
        entity_208: ("&amp;", "&")
        entity_209: ("&euro;", "€")
        entity_210: ("&gt;", ">")
        entity_211: ("&reg;", "®")
        entity_212: ("&#174;", "®")
        entity_213: ("&#xAE;", "®")
        entity_214: ("&quot;", "\"")
        entity_215: ("&#34;", "\"")
        entity_216: ("&#x22;", "\"")
        entity_217: ("&apos;", "'")
        entity_218: ("&#39;", "'")
        entity_219: ("&#x27;", "'")
        entity_220: ("&excl;", "!")
        entity_221: ("&#33;", "!")
        entity_222: ("&num;", "#")
        entity_223: ("&#35;", "#")
        entity_224: ("&dollar;", "$")
        entity_225: ("&#36;", "$")
        entity_226: ("&percnt;", "%")
        entity_227: ("&#37;", "%")
        entity_228: ("&ast;", "*")
        entity_229: ("&#42;", "*")
        entity_230: ("&plus;", "+")
        entity_231: ("&#43;", "+")
        entity_232: ("&comma;", ",")
        entity_233: ("&#44;", ",")
        entity_234: ("&minus;", "−")
        entity_235: ("&#45;", "-")
        entity_236: ("&period;", ".")
        entity_237: ("&#46;", ".")
        entity_238: ("&sol;", "/")
        entity_239: ("&#47;", "/")
        entity_240: ("&colon;", ":")
        entity_241: ("&#58;", ":")
        entity_242: ("&semi;", ";")
        entity_243: ("&#59;", ";")
        entity_244: ("&equals;", "=")
        entity_245: ("&#61;", "=")
        entity_246: ("&quest;", "?")
        entity_247: ("&#63;", "?")
        entity_248: ("&commat;", "@")
        entity_249: ("&#64;", "@")
        entity_250: ("&COPY;", "©")
        entity_251: ("&#128;", "€")
        entity_252: ("&#x9F;", "Ÿ")
        entity_253: ("&#31;", "\u{1f}")
        entity_254: ("&#0;", "�")
        entity_255: ("&#xD800;", "�")
        entity_256: ("&unknownchar;", "&unknownchar;")
        entity_257: ("&#9999999;", "�")
        entity_258: ("&#10;", "\u{a}")
        entity_259: ("&#11;", "\u{b}")
        entity_260: ("&#12;", "\u{c}")
        entity_261: ("&#13;", "\u{d}")
    }
}
