use crate::html5_parser::token_named_characters::TOKEN_NAMED_CHARS;
use crate::html5_parser::token_replacements::TOKEN_REPLACEMENTS;
use crate::html5_parser::tokenizer::Tokenizer;

use super::tokenizer::CHAR_REPLACEMENT;

// All references are to chapters in https://dev.w3.org/html5/spec-LC/tokenization.html

impl<'a> Tokenizer<'a> {
    // Consumes a character reference and places this in the tokenizer consume buffer
    // ref: 8.2.4.69 Tokenizing character references
    pub fn consume_character_reference(&mut self, additional_allowed_char: Option<char>, as_attribute: bool) -> Option<String> {
        self.clear_consume_buffer();

        if as_attribute {
            // When we are inside an attribute context, things (will/might) be different. Not sure how yet.
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
            self.consume('&');
            self.consume(c);
            if self.consume_number().is_err() {
                self.stream.unread();
                return None;
            }

            return Some(self.get_consumed_str());
        }

        // Consume anything else when we found & with another char after (ie: &raquo;)
        self.stream.unread();
        if self.consume_entity(as_attribute).is_err() {
            self.stream.unread();
            return None;
        }

        return Some(self.get_consumed_str());
    }

    // Consume a number like #x1234, #123 etc
    fn consume_number(&mut self) -> Result<String, String> {
        let mut str_num = String::new();

        // Save current position for easy recovery
        let cp = self.stream.tell();

        // Is the char a 'X' or 'x', then we must try and fetch hex digits, otherwise just 0..9
        let mut is_hex = false;
        let hex = match self.stream.look_ahead(0) {
            Some(hex) => hex,
            None => {
                return Err(String::new());
            }
        };

        if hex == 'x' || hex == 'X' {
            is_hex = true;

            // Consume the 'x' character
            let c = match self.stream.read_char() {
                Some(c) => c,
                None => {
                    self.stream.seek(cp);
                    return Err(String::new());
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
                    return Err(String::new());
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
                return Err(String::new());
            }
        };

        // Next character MUST be ;
        if c != ';' {
            self.parse_error("expected a ';'");
            self.stream.seek(cp);
            return Err(String::new());
        }

        self.consume(c);

        // If we found ;. we need to check how many digits we have parsed. It needs to be at least 1,
        if i == 0 {
            self.parse_error("didn't expect #;");
            self.stream.seek(cp);
            return Err(String::new());
        }

        // check if we need to replace the character. First convert the number to a uint, and use that
        // to check if it exists in the replacements table.
        let num = match u32::from_str_radix(&*str_num, if is_hex { 16 } else { 10 }) {
            Ok(n) => n,
            Err(_) => 0,    // lets pretend that an invalid value is set to 0
        };

        if TOKEN_REPLACEMENTS.contains_key(&num) {
            self.clear_consume_buffer();
            self.consume(*TOKEN_REPLACEMENTS.get(&num).unwrap());
            return Ok(String::new());
        }

        // Next, check if we are in the 0xD800..0xDFFF or 0x10FFFF range, if so, replace
        if (num > 0xD800 && num < 0xDFFF) || (num > 0x10FFFFF) {
            self.parse_error("within reserved codepoint range, but replaced");
            self.clear_consume_buffer();
            self.consume(crate::html5_parser::tokenizer::CHAR_REPLACEMENT);
            return Ok(String::new());
        }

        // Check if it's in a reserved range, in that case, we ignore the data
        if self.in_reserved_number_range(num) {
            self.parse_error("within reserved codepoint range, ignored");
            self.clear_consume_buffer();
            return Ok(String::new());
        }

        self.clear_consume_buffer();
        self.consume(std::char::from_u32(num).unwrap_or(CHAR_REPLACEMENT));

        return Ok(String::new());
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

    // This will consume an entity that does not start with &# (ie: &raquo; &#copy;)
    fn consume_entity(&mut self, as_attribute: bool) -> Result<String, String> {

        let mut capture = String::new();

        loop {
            let c = self.stream.read_char();
            match c {
                Some(c) => {
                    capture.push(c);

                    // If we captured [azAZ09], just continue the capture
                    if 'a' <= c && c <= 'z' || 'A' <= c && c <= 'Z' || '0' <= c && c <= '9' {
                        continue;
                    }

                    // If it's something not a ;, we need to unread it
                    if c != ';' {
                        capture.pop();
                    }

                    break;
                }
                None => {
                    self.parse_error("unexpected end of stream");
                    return Err(String::new());
                }
            }
        }

        // At this point, we have a consume buffer with the entity name in it. We need to check if it's a known entity

        if capture.len() == 0 {
            // If we found nohting (ie: &;)
            self.parse_error("expected entity name");
            return Err(String::new());          

        // } else if as_attribute {
            // If we need to consume an entity as an attribute, we need to check if the next character is a valid
            // attribute stuff

        } else if TOKEN_NAMED_CHARS.contains_key(capture.as_str()) {            
            // If we found a known entity, we need to replace it

            let entity = TOKEN_NAMED_CHARS.get(capture.as_str()).unwrap();
            self.consume_string((*entity).to_string());
            return Ok(String::new());

        } else if ! as_attribute {
            // If we found some text, but it's not an entity. We decrease the text until we find something that matches an entity.            
            let mut max_len = capture.len();

            // Largest entity is 6 chars. We don't need to check for more
            if max_len > 6 {
                max_len = 6;
            }

            for j in (1..=max_len).rev() {
                let substr: String = capture.chars().take(j).collect();
                if TOKEN_NAMED_CHARS.contains_key(substr.as_str()) {
                    let entity = TOKEN_NAMED_CHARS.get(substr.as_str()).unwrap();
                    self.consume_string((*entity).to_string());
                    self.consume_string(capture.chars().skip(j).collect());
                    return Ok(String::new());
                }
            }
        }   

        self.consume_string(String::from("????"));
        return Ok(String::new());

        /*
            "&copy;"		-> "(c)"		// case 1: simple entity terminated with ;
            "&copyright;"	-> "(c)"		// case 2: another known entity that takes precedence over the earlier "copy" entity (but happens to be the same returning character)
            "&copynot;"	    -> "(c)not"		// case 3: unknown entity, but &copy is something, so return (c) plus the remainder until ;
            "&copy "		-> "(c)"		// case 4: Terminated by the space, so it's ok
            "&copya"		-> "&copya"		// case 5: Not terminated by a ; (end-of-stream) so "as-is"
            "&copya "		-> "&copya " 	// case 6: Terminated by a space, but not an entity (even though &copy is there), so "as-is"
            "&copy"         -> "&copy"      // case 7: Not terminated by anything (end-of-stream), so "as-is"
        */
    }
}

#[cfg(test)]
mod tests {
    use crate::html5_parser::input_stream::InputStream;
    use super::*;

    macro_rules! token_tests {
        ($($name:ident : $value:expr)*) => {
            $(
                #[test]
                fn $name() {
                    let (input, expected) = $value;

                    let mut is = InputStream::new();
                    is.read_from_str(input, None);
                    let mut tok = Tokenizer::new(&mut is);
                    let t = tok.next_token();
                    assert_eq!(expected, t.to_string());
                }
            )*
        }
    }

    token_tests! {
        // Numbers
        token_0: ("&#10;", "str[\n]")
        token_1: ("&#0;", "str[�]")
        token_2: ("&#x0;", "str[�]")
        token_3: ("&#xdeadbeef;", "str[�]")     // replace with replacement char
        token_4: ("&#xd888;", "str[�]")         // replace with replacement char
        token_5: ("&#xbeef;", "str[뻯]")
        token_6: ("&#x10;", "str[]")                // reserved codepoint
        token_7: ("&#;", "str[&]")
        token_8: ("&;", "str[&]")
        token_9: ("&", "str[&]")
        token_10: ("&#x0001;", "str[]")             // reserved codepoint
        token_11: ("&#x0008;", "str[]")             // reserved codepoint
        token_12: ("&#0008;", "str[]")              // reserved codepoint
        token_13: ("&#8;", "str[]")                 // reserved codepoint
        token_14: ("&#x0009;", "str[\t]")
        token_15: ("&#x007F;", "str[]")             // reserved codepoint
        token_16: ("&#xFDD0;", "str[]")             // reserved codepoint

        // Entities
        token_100: ("&copy;", "str[©]")
        token_101: ("&copyThing;", "str[©Thing;]")
        token_102: ("&raquo;", "str[»]")
        token_103: ("&laquo;", "str[«]")
        token_104: ("&not;", "str[¬]")
        token_105: ("&notit;", "str[¬it;]")
        token_106: ("&notin;", "str[∈]")
        token_107: ("&fo", "str[&fo]")
        token_108: ("&xxx", "str[&xxx]")
        token_109: ("&copy", "str[&copy]")
        token_110: ("&copy ", "str[© ]")
        token_111: ("&copya", "str[©a]")
        token_112: ("&copya;", "str[©a;]")
        token_113: ("&#169;", "str[©]")
        token_114: ("&copy&", "str[©&]")
        token_115: ("&copya ", "str[©&]")
        token_116: ("&#169f ", "str[©f]")


        // ChatGPT generated tests
        token_200: ("&copy;", "str[©]")
        token_201: ("&copy ", "str[©]")
        token_202: ("&#169;", "str[©]")
        token_203: ("&#xA9;", "str[©]")
        token_204: ("&lt;", "str[<]")
        token_205: ("&unknown;", "str[&unknown;]")
        token_206: ("&#60;", "str[<]")
        token_207: ("&#x3C;", "str[<]")
        token_208: ("&amp;", "str[&]")
        token_209: ("&euro;", "str[€]")
        token_210: ("&gt;", "str[>]")
        token_211: ("&reg;", "str[®]")
        token_212: ("&#174;", "str[®]")
        token_213: ("&#xAE;", "str[®]")
        token_214: ("&quot;", "str[\"]")
        token_215: ("&#34;", "str[\"]")
        token_216: ("&#x22;", "str[\"]")
        token_217: ("&apos;", "str[']")
        token_218: ("&#39;", "str[']")
        token_219: ("&#x27;", "str[']")
        token_220: ("&excl;", "str[!]")
        token_221: ("&#33;", "str[!]")
        token_222: ("&num;", "str[#]")
        token_223: ("&#35;", "str[#]")
        token_224: ("&dollar;", "str[$]")
        token_225: ("&#36;", "str[$]")
        token_226: ("&percnt;", "str[%]")
        token_227: ("&#37;", "str[%]")
        token_228: ("&ast;", "str[*]")
        token_229: ("&#42;", "str[*]")
        token_230: ("&plus;", "str[+]")
        token_231: ("&#43;", "str[+]")
        token_232: ("&comma;", "str[,]")
        token_233: ("&#44;", "str[,]")
        token_234: ("&minus;", "str[-]")
        token_235: ("&#45;", "str[-]")
        token_236: ("&period;", "str[.]")
        token_237: ("&#46;", "str[.]")
        token_238: ("&sol;", "str[/]")
        token_239: ("&#47;", "str[/]")
        token_240: ("&colon;", "str[:]")
        token_241: ("&#58;", "str[:]")
        token_242: ("&semi;", "str[;]")
        token_243: ("&#59;", "str[;]")
        token_244: ("&equals;", "str[=]")
        token_245: ("&#61;", "str[=]")
        token_246: ("&quest;", "str[?]")
        token_247: ("&#63;", "str[?]")
        token_248: ("&commat;", "str[@]")
        token_249: ("&#64;", "str[@]")
        token_250: ("&COPY;", "str[&COPY;]")
        token_251: ("&#128;", "str[€]")
        token_252: ("&#x9F;", "str[Ÿ]")
        token_253: ("&#31;", "str[&#31;]")
        token_254: ("&#0;", "str[�]")
        token_255: ("&#xD800;", "str[�]")
        token_256: ("&unknownchar;", "str[&unknownchar;]")
        token_257: ("&#9999999;", "str[�]")
        token_259: ("&#11;", "str[&#11;]")
    }
}
