use crate::html_parser::token_replacements::TOKEN_REPLACEMENTS;
use crate::html_parser::tokenizer::{Tokenizer};

impl Tokenizer {
    // Consumes a character reference and places this in the tokenizer consume buffer
    pub fn consume_character_reference(&mut self, additional_allowed_char: Option<char>) {
        let c = match self.stream.read_char() {
            Some(c) => c,
            None => {
                self.clear_consume_buffer();
                return;
            }
        };

        // If we allow an extra character, check for it
        if additional_allowed_char.is_some() && c == additional_allowed_char.unwrap() {
            self.stream.unread();
            self.clear_consume_buffer();
            return
        }

        match c {
            crate::html_parser::tokenizer::CHAR_TAB |
            crate::html_parser::tokenizer::CHAR_LF |
            crate::html_parser::tokenizer::CHAR_FF => return,
            '#' => self.consume_dash_entity(),
            _ => self.consume_anything_else(),
        }
    }

    // Consume a dash entity #x1234, #123 etc
    fn consume_dash_entity(&mut self) {
        let mut str_num = String::new();

        // Save length for easy recovery
        let len = self.get_consume_len();

        // Consume the dash
        self.consume('#');

        // Is the char a 'X' or 'x', then we must fetch hex digits
        let mut is_hex = false;
        let hex = match self.stream.look_ahead(1) {
            Some(hex) => hex,
            None => {
                self.reset_consume_len(len);
                return
            }
        };

        if hex == 'x' || hex == 'X' {
            is_hex = true;
            // Consume the 'x' character
            let c = match self.stream.read_char() {
                Some(c) => c,
                None => {
                    self.reset_consume_len(len);
                    return
                }
            };

            self.consume(c);
        };

        let mut i = 0;
        loop {
            let c = match self.stream.read_char() {
                Some(c) => c,
                None => {
                    self.reset_consume_len(len);
                    return
                }
            };

            if is_hex && c.is_ascii_hexdigit() {
                str_num.push(c);
                self.consume(c);
            } else if !is_hex && c.is_ascii_digit() {
                str_num.push(c);
                self.consume(c);
            } else {
                break;
            }


            i += 1;
        }

        // Fetch next character
        let c = match self.stream.read_char() {
            Some(c) => c,
            None => {
                self.reset_consume_len(len);
                return
            }
        };

        // Next character MUST be ;
        if c != ';' {
            self.parse_error("expected a ';'");
            self.reset_consume_len(len);
            return
        }

        // If we found ;. we need to check how many digits we have parsed. It needs to be at least 1,
        if i == 0 {
            self.parse_error("didn't expect #;");
            self.reset_consume_len(len);
            return
        }

        // check if we need to replace the character. First convert the number to a uint, and use that
        // to check if it exists in the replacements table.
        let num = match u32::from_str_radix(&*str_num, if is_hex { 16 } else { 10 }) {
            Ok(n) => n,
            Err(_) => 0,    // lets pretend that an invalid value is set to 0
        };

        if TOKEN_REPLACEMENTS.contains_key(&num) {
            self.reset_consume_len(len);
            self.consume(*TOKEN_REPLACEMENTS.get(&num).unwrap());
            return;
        }

        // Next, check if we are in the 0xD800..0xDFFF or 0x10FFFF range, if so, replace
        if (num > 0xD800 && num < 0xDFFF) || (num > 0x10FFFFF) {
            self.reset_consume_len(len);
            self.parse_error("within reserved codepoint range, but replaced");
            self.consume(crate::html_parser::tokenizer::CHAR_REPLACEMENT);
        }

        // Check if it's in a reserved range, in that case, we ignore the data
        if self.in_reserved_number_range(num) {
            self.reset_consume_len(len);
            self.parse_error("within reserved codepoint range, ignored");
        }
    }

    // Returns if the given codepoint number is in a reserved range (as defined in
// https://dev.w3.org/html5/spec-LC/tokenization.html#consume-a-character-reference)
    fn in_reserved_number_range(&self, codepoint: u32) -> bool {
        if
        (0x0001..=0x0008).contains(&codepoint) ||
            (0x000E..=0x001F).contains(&codepoint) ||
            (0x007F..=0x009F).contains(&codepoint) ||
            (0xFDD0..=0xFDEF).contains(&codepoint) ||
            (0x000E..=0x001F).contains(&codepoint) ||
            (0x000E..=0x001F).contains(&codepoint) ||
            (0x000E..=0x001F).contains(&codepoint) ||
            (0x000E..=0x001F).contains(&codepoint) ||
            (0x000E..=0x001F).contains(&codepoint) ||
            [
                0x000B, 0xFFFE, 0xFFFF, 0x1FFFE, 0x1FFFF, 0x2FFFE, 0x2FFFF, 0x3FFFE, 0x3FFFF,
                0x4FFFE, 0x4FFFF, 0x5FFFE, 0x5FFFF, 0x6FFFE, 0x6FFFF, 0x7FFFE, 0x7FFFF,
                0x8FFFE, 0x8FFFF, 0x9FFFE, 0x9FFFF, 0xAFFFE, 0xAFFFF, 0xBFFFE, 0xBFFFF,
                0xCFFFE, 0xCFFFF, 0xDFFFE, 0xDFFFF, 0xEFFFE, 0xEFFFF, 0xFFFFE, 0xFFFFF,
                0x10FFFE, 0x10FFFF
            ].contains(&codepoint) {
            return true;
        }

        return false;
    }

    // This will consume any other matter that does not start with &# (ie: &raquo; &#copy;)
    fn consume_anything_else(&mut self) {}
}