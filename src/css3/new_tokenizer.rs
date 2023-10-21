// note: input_stream should come from a shared lib.
use crate::html5::input_stream::InputStream;

use crate::css3::unicode::{get_unicode_char, UnicodeChar};
use std::usize;

#[derive(Debug, PartialEq)]
pub enum NumberKind {
    Number,
    Integer,
}

pub type Number = f32;

// todo: add def for each token
#[derive(Debug, PartialEq)]
pub enum Token {
    /// A [`<at-keyword-token>`](https://drafts.csswg.org/css-syntax/#at-keyword-token-diagram)
    ///
    /// The value does not include the `@` marker.
    AtKeyword(String),
    Ident(String),
    Function(String),
    Url(String),
    BadUrl(String),
    Dimension {
        unit: String,
        value: Number,
    },
    Percentage(Number),
    Number(Number),
    /// A [`<string-token>`](https://drafts.csswg.org/css-syntax/#string-token-diagram)
    ///
    /// The value does not include the quotes.
    QuotedString(String),
    /// A `<bad-string-token>`
    ///
    /// This token always indicates a parse error.
    BadString(String),
    /// A [`<whitespace-token>`](https://drafts.csswg.org/css-syntax/#whitespace-token-diagram)
    Whitespace,
    /// A [`<hash-token>`](https://drafts.csswg.org/css-syntax/#hash-token-diagram) with the type flag set to "unrestricted"
    ///
    /// The value does not include the `#` marker.
    Hash(String),
    /// A [`<hash-token>`](https://drafts.csswg.org/css-syntax/#hash-token-diagram) with the type flag set to "id"
    ///
    /// The value does not include the `#` marker.
    ///
    /// Hash that is a valid ID selector.
    IDHash(String),
    /// A `<delim-token>`
    Delim(char),
    /// A `<}-token>`
    LCurly,
    /// A `<{-token>`
    RCurly,
    /// A `<(-token>`
    LParen,
    /// A `<)-token>`
    RParen,
    /// A `<]-token>`
    LBracket,
    /// A `<[-token>`
    RBracket,
    /// A `<comma-token>`
    Comma,
    /// A `:` `<colon-token>`
    Colon,
    /// A `;` `<semicolon-token>`
    Semicolon,
    // A `<!--` `<CDO-token>`
    CDO,
    // A `-->` `<CDC-token>`
    CDC,
    // A `<EOF-token>`
    EOF,
}

macro_rules! consume {
    ($self:expr, $token:expr) => {{
        $self.stream.read_char();

        $token
    }};
}

/// CSS Tokenizer according to the [w3 specification](https://www.w3.org/TR/css-syntax-3/#tokenization)
pub struct Tokenizer<'stream> {
    pub stream: &'stream mut InputStream,
    /// Next token that has been consumed.
    pub current_token: Option<String>,
    /// Next token that has not yet been consumed
    pub next_token: Option<String>,
}

impl<'stream> Tokenizer<'stream> {
    pub fn new(stream: &'stream mut InputStream) -> Tokenizer {
        Tokenizer {
            stream,
            current_token: None,
            next_token: None,
        }
    }

    /// 4.3.1. [Consume a token](https://www.w3.org/TR/css-syntax-3/#consume-token)
    pub fn consume_token(&mut self) -> Token {
        self.consume_comment();

        // todo: reframe the concept of "tokenizer::current" and "is::current" and "is::next"
        let current = self.stream.current_char().utf8();

        match current {
            c if c.is_whitespace() => self.consume_whitespace(),
            // note: consume_string_token doesn't work as expected
            '"' | '\'' => self.consume_string_token(),
            '#' => {
                // start=1 => skip '#' when checking for escaped sequence
                if self.is_ident_char(self.stream.next_char().utf8()) || self.is_start_of_escape(1)
                {
                    // consume '#'
                    self.stream.read_char();
                    return Token::IDHash(self.consume_ident());
                }

                Token::Delim(current)
            }
            ')' => consume!(self, Token::RParen),
            '(' => consume!(self, Token::LParen),
            '[' => consume!(self, Token::LBracket),
            ']' => consume!(self, Token::RBracket),
            '{' => consume!(self, Token::LCurly),
            '}' => consume!(self, Token::RCurly),
            ',' => consume!(self, Token::Comma),
            ':' => consume!(self, Token::Colon),
            ';' => consume!(self, Token::Semicolon),
            '+' | '.' => {
                if self.stream.next_char().utf8().is_numeric() {
                    return self.consume_numeric_token();
                }

                // consume '+'
                self.stream.read_char();
                Token::Delim(current)
            }
            '-' => {
                if self.stream.next_char().utf8().is_numeric() {
                    return self.consume_numeric_token();
                }

                if self.stream.look_ahead_slice(3) == "-->" {
                    return Token::CDC;
                }

                if self.is_ident_start(current) {
                    return self.consume_ident_like_seq();
                }

                Token::Delim(current)
            }
            '<' => {
                let cdo_token = "<!--";
                if self.stream.look_ahead_slice(cdo_token.len()) == cdo_token {
                    // consume "<!--"
                    self.consume_chars(cdo_token.len());
                    return Token::CDO;
                }

                // consume '<'
                self.stream.read_char();
                Token::Delim(current)
            }
            '@' => {
                // consume '@'
                self.stream.read_char();

                if self.is_ident_start(self.stream.next_char().utf8()) {
                    return Token::AtKeyword(self.consume_ident());
                }

                Token::Delim(current)
            }
            '\\' => {
                if self.is_start_of_escape(0) {
                    return self.consume_ident_like_seq();
                }

                // parser error
                // consume '\'
                self.stream.read_char();
                Token::Delim(current)
            }
            c if c.is_numeric() => self.consume_numeric_token(),
            c if self.is_ident_start(c) => self.consume_ident_like_seq(),
            _ => {
                let el = self.stream.read_char();
                if el.is_eof() {
                    return Token::EOF;
                }
                Token::Delim(el.utf8())
            }
        }
    }

    /// 4.3.2. [Consume comments](https://www.w3.org/TR/css-syntax-3/#consume-comment)
    pub fn consume_comment(&mut self) {
        if self.stream.look_ahead_slice(2) == "/*" {
            while self.stream.look_ahead_slice(2) != "*/" {
                self.stream.read_char();
            }

            // consume '*/'
            self.consume_chars(2);
        };
    }

    /// 4.3.3. [Consume a numeric token]()
    /// Returns either a `<number-token>`, `<percentage-token>`, or `<dimension-token>`.
    pub fn consume_numeric_token(&mut self) -> Token {
        let number = self.consume_number();

        if self.is_ident_start(self.stream.current_char().utf8()) {
            let unit = self.consume_ident();

            return Token::Dimension {
                unit,
                value: number,
            };
        } else if self.stream.current_char().utf8() == '%' {
            return Token::Percentage(number);
        }

        Token::Number(number)
    }

    /// 4.3.5. [Consume a string token](https://www.w3.org/TR/css-syntax-3/#consume-string-token)
    ///
    /// Returns either a `<string-token>` or `<bad-string-token>`.
    pub fn consume_string_token(&mut self) -> Token {
        // consume string staring: (') or (") ...
        let ending = self.stream.read_char().utf8();
        let mut value = String::new();

        loop {
            // if eof => parser error => return the current string
            if self.stream.current_char().utf8() == ending || self.stream.eof() {
                // consume string ending
                self.stream.read_char();
                return Token::QuotedString(value);
            }

            // newline: parser error
            if self.stream.current_char().utf8() == '\n' {
                // note: don't consume '\n'
                return Token::BadString(value);
            }

            if self.stream.current_char().utf8() == '\\' && self.stream.next_char().utf8() == '\n' {
                // consume '\'
                self.stream.read_char();
                continue;
            }

            if self.is_start_of_escape(0) {
                value.push(self.consume_escaped_token());
                continue;
            }

            value.push(self.stream.read_char().utf8())
        }
    }

    /// 4.3.12. [Consume a number](https://www.w3.org/TR/css-syntax-3/#consume-number)
    ///
    /// Note: for the sake of simplicity, we exclude the number type mentioned in the algorithm.
    pub fn consume_number(&mut self) -> Number {
        let mut value = String::new();
        let lookahead = self.stream.current_char().utf8();

        if lookahead == '+' || lookahead == '-' {
            value.push(self.stream.read_char().utf8());
        }

        value.push_str(&self.consume_digits());

        if self.stream.current_char().utf8() == '.' && self.stream.next_char().utf8().is_numeric() {
            value.push_str(&self.consume_chars(2));
        }

        // type should be "number"
        value.push_str(&self.consume_digits());

        // todo: move them to gobal constants
        // U+0045: LATIN CAPITAL LETTER E (E)
        // U+0065: LATIN SMALL LETTER E (e)
        if self.stream.current_char().utf8() == '\u{0045}'
            || self.stream.current_char().utf8() == '\u{0065}'
        {
            value.push(self.stream.read_char().utf8());

            if self.stream.current_char().utf8() == '-' || self.stream.current_char().utf8() == '+'
            {
                value.push(self.stream.read_char().utf8());
            }
        }

        value.push_str(&self.consume_digits());

        value.parse().expect("failed to parse number")
    }

    /// 4.3.4. [Consume an ident-like token](https://www.w3.org/TR/css-syntax-3/#consume-ident-like-token)
    ///
    /// Returns: `<ident-token>`, `<function-token>`, `<url-token>`, or `<bad-url-token>`.
    pub fn consume_ident_like_seq(&mut self) -> Token {
        let value = self.consume_ident();

        if value == "url" && self.stream.current_char().utf8() == '(' {
            // consume '('
            self.stream.read_char();
            self.consume_whitespace();

            if self.is_any_of(vec!['"', '\'']) {
                return Token::Function(value);
            }

            return self.consume_url();
        } else if self.stream.current_char().utf8() == '(' {
            // consume '('
            self.stream.read_char();
            return Token::Function(value);
        }

        Token::Ident(value)
    }

    /// 4.3.6. [Consume a url token](https://www.w3.org/TR/css-syntax-3/#consume-a-url-token)
    ///
    /// Returns either a `<url-token>` or a `<bad-url-token>`
    pub fn consume_url(&mut self) -> Token {
        let mut url = String::new();

        self.consume_whitespace();

        loop {
            if self.stream.current_char().utf8() == ')' {
                // consume ')'
                self.stream.read_char();
                break;
            }

            if self.stream.eof() {
                // parser error
                break;
            }

            if self.stream.current_char().utf8().is_whitespace() {
                self.consume_whitespace();
                continue;
            }

            if self.is_any_of(vec!['"', '\'', '(']) || self.is_non_printable_char() {
                // parse error
                self.consume_remnants_of_bad_url();
                return Token::BadUrl(url);
            }

            if self.is_start_of_escape(0) {
                url.push(self.consume_escaped_token());
                continue;
            }

            url.push(self.stream.read_char().utf8());
        }

        Token::Url(url)
    }

    /// 4.3.14. [Consume the remnants of a bad url](https://www.w3.org/TR/css-syntax-3/#consume-remnants-of-bad-url)
    ///
    /// Used is to consume enough of the input stream to reach a recovery point where normal tokenizing can resume.
    fn consume_remnants_of_bad_url(&mut self) {
        loop {
            // recovery point
            if self.stream.current_char().utf8() == ')' || self.stream.eof() {
                break;
            }

            if self.is_start_of_escape(0) {
                self.consume_escaped_token();
            }

            // todo: parse escaped code point.
            self.stream.read_char();
        }
    }

    /// 4.3.7. [Consume an escaped code point](https://www.w3.org/TR/css-syntax-3/#consume-an-escaped-code-point)
    pub fn consume_escaped_token(&mut self) -> char {
        // consume '\'
        self.stream.read_char();

        let mut value = String::new();

        let default_char = get_unicode_char(UnicodeChar::ReplacementCharacter);
        // eof: parser error
        if self.stream.eof() {
            return default_char;
        }

        while self.stream.current_char().utf8().is_ascii_hexdigit() && value.len() <= 5 {
            value.push(self.stream.read_char().utf8());
        }

        if self.stream.current_char().utf8().is_whitespace() {
            self.stream.read_char();
        }

        let as_u32 = u32::from_str_radix(&value, 16).expect("unable to parse hex string as number");

        // todo: look for better implementation
        if let Some(char) = char::from_u32(as_u32) {
            if char == get_unicode_char(UnicodeChar::Null)
                || char >= get_unicode_char(UnicodeChar::MaxAllowed)
            {
                return default_char;
            }

            return char;
        }

        default_char
    }

    /// 4.3.11. [Consume an ident
    /// sequence](https://www.w3.org/TR/css-syntax-3/#consume-name)
    ///
    /// Note: that algorithm does not do the verification that are necessary to
    /// ensure the returned code points would constitute an <ident-token>.
    /// Caller should ensure that the stream starts with an ident sequence before calling this
    /// algorithm.
    pub fn consume_ident(&mut self) -> String {
        let mut value = String::new();

        while self.is_ident_char(self.stream.current_char().utf8()) {
            value.push(self.stream.read_char().utf8());
            // todo: Consume an escaped code point.
        }

        value
    }

    pub fn consume_digits(&mut self) -> String {
        let mut value = String::new();

        while self.stream.current_char().utf8().is_numeric() {
            value.push(self.stream.read_char().utf8())
        }

        value
    }

    pub fn consume_chars(&mut self, mut len: usize) -> String {
        let mut value = String::new();

        while len > 0 {
            value.push(self.stream.read_char().utf8());
            len -= 1
        }

        value
    }

    fn consume_whitespace(&mut self) -> Token {
        while self.stream.current_char().utf8().is_whitespace() {
            self.stream.read_char();
        }

        Token::Whitespace
    }

    fn is_ident_start(&self, char: char) -> bool {
        char.is_alphabetic() || !char.is_ascii() || char == get_unicode_char(UnicodeChar::LowLine)
    }

    fn is_ident_char(&self, char: char) -> bool {
        self.is_ident_start(char) || char.is_numeric() || char == '\u{002D}' // ??
    }

    /// def: [non-printable code point](https://www.w3.org/TR/css-syntax-3/#non-printable-code-point)
    fn is_non_printable_char(&self) -> bool {
        let char = self.stream.current_char().utf8();

        (char >= get_unicode_char(UnicodeChar::Null)
            && char <= get_unicode_char(UnicodeChar::Backspace))
            || (char >= get_unicode_char(UnicodeChar::ShiftOut)
                && char <= get_unicode_char(UnicodeChar::InformationSeparatorOne))
            || char == get_unicode_char(UnicodeChar::Tab)
            || char == get_unicode_char(UnicodeChar::Delete)
    }

    /// 4.3.8. [Check if two code points are a valid escape](https://www.w3.org/TR/css-syntax-3/#starts-with-a-valid-escape)
    fn is_start_of_escape(&self, start: usize) -> bool {
        let current_char = self.stream.look_ahead(start);
        let next_char = self.stream.look_ahead(start + 1);

        current_char.utf8() == '\\' && next_char.utf8() != '\n'
    }

    fn is_any_of(&self, chars: Vec<char>) -> bool {
        let current_char = self.stream.current_char().utf8();
        for char in chars {
            if current_char == char {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::html5::input_stream::Encoding;

    #[test]
    fn parse_comment() {
        let mut is = InputStream::new();
        is.read_from_str("/* css comment */", Some(Encoding::UTF8));

        let mut tokenizer = Tokenizer::new(&mut is);
        tokenizer.consume_comment();

        assert!(is.eof())
    }

    #[test]
    fn parse_numbers() {
        let mut is = InputStream::new();

        let num_tokens = vec![
            ("12", 12.0),
            ("+34", 34.0),
            ("-56", -56.0),
            ("7.8", 7.8),
            ("-9.10", -9.10),
            ("0.0001", 0.0001),
            ("1e+1", 1e+1),
            ("1e1", 1e1),
            ("1e-1", 1e-1),
        ];

        let mut tokenizer = Tokenizer::new(&mut is);

        for (raw_num, num_token) in num_tokens {
            tokenizer
                .stream
                .read_from_str(raw_num, Some(Encoding::UTF8));
            assert_eq!(tokenizer.consume_number(), num_token);
        }
    }

    // todo: add more tests for the `<ident-token>`
    #[test]
    fn parse_ident_tokens() {
        let mut is = InputStream::new();

        let ident_tokens = vec![
            ("-ident", "-ident"),
            ("ide  nt", "ide"),
            ("_123-ident", "_123-ident"),
            ("_123\\ident", "_123"),
        ];

        let mut tokenizer = Tokenizer::new(&mut is);

        for (raw_ident, ident_tokens) in ident_tokens {
            tokenizer
                .stream
                .read_from_str(raw_ident, Some(Encoding::UTF8));

            assert_eq!(tokenizer.consume_ident(), ident_tokens);
        }
    }

    #[test]
    fn parse_escaped_tokens() {
        {
            let mut is = InputStream::new();

            let escaped_chars = vec![
                ("\\005F ", get_unicode_char(UnicodeChar::LowLine)),
                ("\\2A", '*'),
                (
                    "\\000000 ",
                    get_unicode_char(UnicodeChar::ReplacementCharacter),
                ),
                (
                    "\\FFFFFF ",
                    get_unicode_char(UnicodeChar::ReplacementCharacter),
                ),
                (
                    "\\10FFFF ",
                    get_unicode_char(UnicodeChar::ReplacementCharacter),
                ),
            ];

            let mut tokenizer = Tokenizer::new(&mut is);

            for (raw_escaped, escaped_char) in escaped_chars {
                tokenizer
                    .stream
                    .read_from_str(raw_escaped, Some(Encoding::UTF8));
                assert_eq!(tokenizer.consume_escaped_token(), escaped_char);
            }
        }
    }

    #[test]
    fn parse_urls() {
        let mut is = InputStream::new();

        let urls = vec![
            (
                "url(https://gosub.io/)",
                Token::Url("https://gosub.io/".into()),
            ),
            ("url(  gosub.io   )", Token::Url("gosub.io".into())),
            ("url(gosub\u{002E}io)", Token::Url("gosub.io".into())),
            ("url(gosub\u{FFFD}io)", Token::Url("gosub�io".into())),
            ("url(gosub\u{0000}io)", Token::BadUrl("gosub".into())),
        ];

        let mut tokenizer = Tokenizer::new(&mut is);

        for (raw_url, url_token) in urls {
            tokenizer
                .stream
                .read_from_str(raw_url, Some(Encoding::UTF8));
            assert_eq!(tokenizer.consume_ident_like_seq(), url_token);
        }
    }

    #[test]
    fn parse_function_tokens() {
        let mut is = InputStream::new();

        let functions = vec![
            ("url(\"", Token::Function("url".into())),
            ("url( \"", Token::Function("url".into())),
            ("url(\'", Token::Function("url".into())),
            ("url( \'", Token::Function("url".into())),
            ("url(\"", Token::Function("url".into())),
            ("attr('", Token::Function("attr".into())),
            ("rotateX(    '", Token::Function("rotateX".into())),
            ("rotateY(    \"", Token::Function("rotateY".into())),
        ];

        let mut tokenizer = Tokenizer::new(&mut is);

        for (raw_function, function_token) in functions {
            tokenizer
                .stream
                .read_from_str(raw_function, Some(Encoding::UTF8));
            assert_eq!(tokenizer.consume_ident_like_seq(), function_token);
        }
    }

    #[test]
    fn parser_numeric_token() {
        let mut is = InputStream::new();

        let numeric_tokens = vec![
            (
                "1.1rem",
                Token::Dimension {
                    value: 1.1,
                    unit: "rem".into(),
                },
            ),
            (
                "1px",
                Token::Dimension {
                    value: 1.0,
                    unit: "px".into(),
                },
            ),
            ("100%", Token::Percentage(100.0)),
            ("42", Token::Number(42.0)),
            ("18 px", Token::Number(18.0)),
        ];

        let mut tokenizer = Tokenizer::new(&mut is);

        for (raw_token, token) in numeric_tokens {
            tokenizer
                .stream
                .read_from_str(raw_token, Some(Encoding::UTF8));
            assert_eq!(tokenizer.consume_numeric_token(), token);
        }
    }

    #[test]
    fn parse_string_tokens() {
        let mut is = InputStream::new();

        let string_tokens = vec![
            ("'line\nnewline'", Token::BadString("line".into())),
            (
                "\"double quotes\"",
                Token::QuotedString("double quotes".into()),
            ),
            (
                "\'single quotes\'",
                Token::QuotedString("single quotes".into()),
            ),
            ("#hash#", Token::QuotedString("hash".into())),
            ("\"eof", Token::QuotedString("eof".into())),
            ("\"\"", Token::QuotedString("".into())),
        ];

        let mut tokenizer = Tokenizer::new(&mut is);

        for (raw_string, string_token) in string_tokens {
            tokenizer
                .stream
                .read_from_str(raw_string, Some(Encoding::UTF8));
            assert_eq!(tokenizer.consume_string_token(), string_token);
        }
    }
}
