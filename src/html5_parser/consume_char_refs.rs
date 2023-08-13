use crate::html5_parser::token_named_characters::TOKEN_NAMED_CHARS;
use crate::html5_parser::token_replacements::TOKEN_REPLACEMENTS;
use crate::html5_parser::tokenizer::Tokenizer;

// All references are to chapters in https://dev.w3.org/html5/spec-LC/tokenization.html

impl<'a> Tokenizer<'a> {
    // Consumes a character reference and places this in the tokenizer consume buffer
    // ref: 8.2.4.69 Tokenizing character references
    pub fn consume_character_reference(&mut self, additional_allowed_char: Option<char>, as_attribute: bool) -> Option<String> {
        self.clear_consume_buffer();
        self.consume('&');

        if as_attribute {
        }

        let c = match self.stream.read_char() {
            Some(c) => c,
            None => {
                return None;
            }
        };

        // Characters that aren't allowed
        let mut chars = vec![
            crate::html5_parser::tokenizer::CHAR_TAB,
            crate::html5_parser::tokenizer::CHAR_LF,
            crate::html5_parser::tokenizer::CHAR_FF,
            crate::html5_parser::tokenizer::CHAR_SPACE,
            '<',
            '&'
        ];

        // The name is weird: addiitonal_allowed_chars, but it would be a char that is NOT allowed (?)
        if additional_allowed_char.is_some() {
            chars.push(additional_allowed_char.unwrap())
        }

        if chars.contains(&c) {
            self.stream.unread();
            return None;
        }

        // Consume a number when we found &#
        if c == '#' {
            self.consume(c);
            match self.consume_number() {
                Some(_) => {}
                None => {
                    self.stream.unread();
                    return None;
                }
            }

            return Some(self.get_consumed_str());
        }

        // Consume anything else when we found & with another char after (ie: &raquo;)
        match self.consume_anything_else() {
            Some(_) => {}
            None => {
                self.stream.unread();
                return None;
            }
        }
        return Some(self.get_consumed_str());
    }

    // Consume a number like #x1234, #123 etc
    fn consume_number(&mut self) -> Option<String> {
        let mut str_num = String::new();

        // Save current position for easy recovery
        let cp = self.stream.tell();

        // Is the char a 'X' or 'x', then we must try and fetch hex digits, otherwise just 0..9
        let mut is_hex = false;
        let hex = match self.stream.look_ahead(0) {
            Some(hex) => hex,
            None => {
                return None
            }
        };

        if hex == 'x' || hex == 'X' {
            is_hex = true;

            // Consume the 'x' character
            let c = match self.stream.read_char() {
                Some(c) => c,
                None => {
                    self.stream.seek(cp);
                    return None
                }
            };

            self.consume(c);
        };

        let mut i = 0;
        loop {
            let c = match self.stream.read_char() {
                Some(c) => c,
                None => {
                    self.stream.seek(cp);
                    return None
                }
            };

            if is_hex && c.is_ascii_hexdigit() {
                str_num.push(c);
                self.consume(c);
            } else if !is_hex && c.is_ascii_digit() {
                str_num.push(c);
                self.consume(c);
            } else {
                self.stream.unread();
                break;
            }

            i += 1;
        }

        // Fetch next character
        let c = match self.stream.read_char() {
            Some(c) => c,
            None => {
                self.stream.seek(cp);
                return None
            }
        };

        // Next character MUST be ;
        if c != ';' {
            self.parse_error("expected a ';'");
            self.stream.seek(cp);
            return None
        }

        self.consume(c);

        // If we found ;. we need to check how many digits we have parsed. It needs to be at least 1,
        if i == 0 {
            self.parse_error("didn't expect #;");
            self.stream.seek(cp);
            return None
        }

        // check if we need to replace the character. First convert the number to a uint, and use that
        // to check if it exists in the replacements table.
        let num = match u32::from_str_radix(&*str_num, if is_hex { 16 } else { 10 }) {
            Ok(n) => n,
            Err(_) => 0,    // lets pretend that an invalid value is set to 0
        };

        if TOKEN_REPLACEMENTS.contains_key(&num) {
            // self.stream.seek(cp);
            let s = *TOKEN_REPLACEMENTS.get(&num).unwrap();
            return Some(String::from(s));
        }

        // Next, check if we are in the 0xD800..0xDFFF or 0x10FFFF range, if so, replace
        if (num > 0xD800 && num < 0xDFFF) || (num > 0x10FFFFF) {
            self.parse_error("within reserved codepoint range, but replaced");
            self.clear_consume_buffer();
            self.consume(crate::html5_parser::tokenizer::CHAR_REPLACEMENT);
        }

        // Check if it's in a reserved range, in that case, we ignore the data
        if self.in_reserved_number_range(num) {
            self.parse_error("within reserved codepoint range, ignored");
            self.clear_consume_buffer();
        }

        return Some(self.get_consumed_str());
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
    fn consume_anything_else(&mut self) -> Option<String> {
        let mut match_str = String::from("");
        let mut current_found_match = String::from("");

        let mut tmp = String::new();

        loop {
            let c = match self.stream.read_char() {
                Some(c) => c,
                None => {
                    break;
                }
            };

            if c == ';' {
                break;
            }

            match_str.push(c);

            // If we match "not" we can safely set this. If later we match "notit", that will override
            // the current_match_found.
            if TOKEN_NAMED_CHARS.contains_key(&*match_str) {
                current_found_match = match_str.clone()
            }
        }

        tmp = tmp + &current_found_match;
        tmp.push(';');

        return Some(tmp)
    }
}

#[cfg(test)]
mod tests {
    use crate::html_parser::input_stream::InputStream;
    use super::*;

    #[test]
    fn test_consume_character_reference() {
        let mut is = InputStream::new();
        is.read_from_str("&#10;", None);
        let mut tok = Tokenizer::new(&mut is);
        tok.next_token();
        // // assert_eq!(t, Token::String);

        let mut is = InputStream::new();
        is.read_from_str("&#x124;", None);
        let mut tok = Tokenizer::new(&mut is);
        tok.next_token();
        // // assert_eq!(t, Token::String);

        let mut is = InputStream::new();
        is.read_from_str("&#x80;", None);
        let mut tok = Tokenizer::new(&mut is);
        tok.next_token();
        // // assert_eq!(t, Token::String);

        let mut is = InputStream::new();
        is.read_from_str("&not;", None);
        let mut tok = Tokenizer::new(&mut is);
        tok.next_token();
        // // assert_eq!(t, Token::String);

        let mut is = InputStream::new();
        is.read_from_str("&notit;", None);
        let mut tok = Tokenizer::new(&mut is);
        tok.next_token();
        // assert_eq!(t, Token::String);
    }
}
