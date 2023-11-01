// note: input_stream should come from a shared lib.
use crate::css3::unicode::{get_unicode_char, UnicodeChar};
use crate::html5::input_stream::{
    Bytes::{self, *},
    InputStream,
};
use std::usize;

pub type Number = f32;

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum TokenKind {
    AtKeyword,
    Ident,
    Function,
    Url,
    BadUrl,
    Dimension,
    Percentage,
    Number,
    QuotedString,
    BadString,
    Whitespace,
    Hash,
    IDHash,
    Delim,
    LCurly,
    RCurly,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Colon,
    Semicolon,
    Comma,
    CDO,
    CDC,
    EOF,
    // Match any token
    Any,
}

// todo: add def for each token
#[derive(Debug, PartialEq, Clone)]
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

impl Token {
    pub fn kind(&self) -> TokenKind {
        match self {
            Token::AtKeyword(_) => TokenKind::AtKeyword,
            Token::QuotedString(_) => TokenKind::QuotedString,
            Token::BadString(_) => TokenKind::BadString,
            Token::BadUrl(_) => TokenKind::BadUrl,
            Token::Url(_) => TokenKind::Url,
            Token::Delim(_) => TokenKind::Delim,
            Token::Ident(_) => TokenKind::Ident,
            Token::Function(_) => TokenKind::Function,
            Token::Dimension { .. } => TokenKind::Dimension,
            Token::Percentage(_) => TokenKind::Percentage,
            Token::Number(_) => TokenKind::Number,
            Token::Hash(_) => TokenKind::Hash,
            Token::IDHash(_) => TokenKind::IDHash,
            Token::Whitespace => TokenKind::Whitespace,
            Token::LCurly => TokenKind::LCurly,
            Token::RCurly => TokenKind::RCurly,
            Token::LBracket => TokenKind::LBracket,
            Token::RBracket => TokenKind::RBracket,
            Token::LParen => TokenKind::LParen,
            Token::RParen => TokenKind::RParen,
            Token::Colon => TokenKind::Colon,
            Token::Semicolon => TokenKind::Semicolon,
            Token::Comma => TokenKind::Comma,
            Token::CDO => TokenKind::CDO,
            Token::CDC => TokenKind::CDC,
            Token::EOF => TokenKind::EOF,
        }
    }

    pub fn is_eof(&self) -> bool {
        self.kind() == TokenKind::EOF
    }

    pub fn value(&self) -> String {
        match self {
            Token::Function(val) => val.clone(),
            Token::AtKeyword(val) => val.clone(),
            Token::QuotedString(val) => val.clone(),
            Token::BadString(val) => val.clone(),
            Token::BadUrl(val) => val.clone(),
            Token::Url(val) => val.clone(),
            Token::Delim(val) => val.to_string(),
            Token::Ident(val) => val.clone(),
            Token::Dimension { unit, .. } => unit.clone(),
            Token::Percentage(val) => val.to_string(),
            Token::Number(val) => val.to_string(),
            Token::Hash(val) => val.clone(),
            Token::IDHash(val) => val.clone(),
            _ => String::new(),
        }
    }
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
    /// Current position
    position: usize,
    /// Full list of all tokens produced by the tokenizer
    tokens: Vec<Token>,
}

impl<'stream> Tokenizer<'stream> {
    pub fn new(stream: &'stream mut InputStream) -> Tokenizer {
        Tokenizer {
            stream,
            position: 0,
            tokens: Vec::new(),
        }
    }

    pub fn consume_all(&mut self) {
        while !self.stream.eof() {
            let token = self.consume_token();
            self.tokens.push(token);
        }

        self.position = 0;
    }

    pub fn lookahead(&self, offset: usize) -> Token {
        if self.position + offset >= self.tokens.len() {
            return Token::EOF;
        }

        self.tokens[self.position + offset].clone()
    }

    pub fn consume(&mut self) -> Token {
        let token = &self.tokens[self.position];
        self.position += 1;
        token.clone()
    }

    /// 4.3.1. [Consume a token](https://www.w3.org/TR/css-syntax-3/#consume-token)
    pub fn consume_token(&mut self) -> Token {
        self.consume_comment();

        // todo: reframe the concept of "tokenizer::current" and "is::current" and "is::next"
        let current = self.stream.current_char();

        match current {
            Ch(c) if c.is_whitespace() => self.consume_whitespace(),
            // note: consume_string_token doesn't work as expected
            Ch('"' | '\'') => self.consume_string_token(),
            Ch(c @ '#') => {
                // consume '#'
                self.stream.read_char();

                if self.is_ident_char(self.stream.current_char().into())
                    || self.is_start_of_escape(0)
                {
                    if self.is_next_3_points_starts_ident_seq(0) {
                        return Token::IDHash(self.consume_ident());
                    } else {
                        return Token::Hash(self.consume_ident());
                    }
                }

                Token::Delim(c)
            }
            Ch(')') => consume!(self, Token::RParen),
            Ch('(') => consume!(self, Token::LParen),
            Ch('[') => consume!(self, Token::LBracket),
            Ch(']') => consume!(self, Token::RBracket),
            Ch('{') => consume!(self, Token::LCurly),
            Ch('}') => consume!(self, Token::RCurly),
            Ch(',') => consume!(self, Token::Comma),
            Ch(':') => consume!(self, Token::Colon),
            Ch(';') => consume!(self, Token::Semicolon),
            Ch(c @ '+') => {
                if self.is_signed_decimal(0) {
                    return self.consume_numeric_token();
                }

                // consume '+'
                self.stream.read_char();
                Token::Delim(c)
            }
            Ch('.') => {
                if matches!(self.stream.next_char(), Ch(c) if c.is_numeric()) {
                    return self.consume_numeric_token();
                }

                // consume '.'
                self.stream.read_char();
                Token::Delim('.')
            }
            Ch(c @ '-') => {
                if self.is_signed_decimal(0) {
                    return self.consume_numeric_token();
                }

                let cdc_token = "-->";
                if self.stream.look_ahead_slice(cdc_token.len()) == cdc_token {
                    // consume '--'
                    self.consume_chars(cdc_token.len());
                    return Token::CDC;
                }

                if self.is_next_3_points_starts_ident_seq(0) {
                    return self.consume_ident_like_seq();
                }

                // consume '-'
                self.stream.read_char();
                Token::Delim(c)
            }
            Ch(c @ '<') => {
                let cdo_token = "<!--";
                if self.stream.look_ahead_slice(cdo_token.len()) == cdo_token {
                    // consume "<!--"
                    self.consume_chars(cdo_token.len());
                    return Token::CDO;
                }

                // consume '<'
                self.stream.read_char();
                Token::Delim(c)
            }
            Ch(c @ '@') => {
                // consume '@'
                self.stream.read_char();

                if self.is_next_3_points_starts_ident_seq(0) {
                    return Token::AtKeyword(self.consume_ident());
                }

                Token::Delim(c)
            }
            Ch(c @ '\\') => {
                if self.is_start_of_escape(0) {
                    return self.consume_ident_like_seq();
                }

                // parser error
                // consume '\'
                self.stream.read_char();
                Token::Delim(c)
            }
            Ch(c) if c.is_numeric() => self.consume_numeric_token(),
            Ch(c) if self.is_ident_start(c) => self.consume_ident_like_seq(),
            _ => {
                let c = self.stream.read_char();
                if matches!(c, Bytes::Eof) {
                    return Token::EOF;
                }
                Token::Delim(c.into())
            }
        }
    }

    /// 4.3.2. [Consume comments](https://www.w3.org/TR/css-syntax-3/#consume-comment)
    pub fn consume_comment(&mut self) -> String {
        let mut comment = String::new();
        if self.stream.look_ahead_slice(2) == "/*" {
            // consume '/*'
            comment.push_str(&self.consume_chars(2));

            while self.stream.look_ahead_slice(2) != "*/" && !self.stream.eof() {
                comment.push(self.stream.read_char().into());
            }

            // consume '*/'
            comment.push_str(&self.consume_chars(2));
        };

        comment
    }

    /// 4.3.3. [Consume a numeric token]()
    /// Returns either a `<number-token>`, `<percentage-token>`, or `<dimension-token>`.
    pub fn consume_numeric_token(&mut self) -> Token {
        let number = self.consume_number();

        if self.is_next_3_points_starts_ident_seq(0) {
            let unit = self.consume_ident();

            return Token::Dimension {
                unit,
                value: number,
            };
        } else if self.stream.current_char() == Ch('%') {
            // consume '%'
            self.stream.read_char();
            return Token::Percentage(number);
        }

        Token::Number(number)
    }

    /// 4.3.5. [Consume a string token](https://www.w3.org/TR/css-syntax-3/#consume-string-token)
    ///
    /// Returns either a `<string-token>` or `<bad-string-token>`.
    pub fn consume_string_token(&mut self) -> Token {
        // consume string staring: (') or (") ...
        let ending = self.stream.read_char();
        let mut value = String::new();

        loop {
            // if eof => parser error => return the current string
            if self.stream.current_char() == ending || self.stream.eof() {
                // consume string ending
                self.stream.read_char();
                return Token::QuotedString(value);
            }

            // newline: parser error
            if self.stream.current_char() == Ch('\n') {
                // note: don't consume '\n'
                return Token::BadString(value);
            }

            if self.stream.current_char() == Ch('\\') && self.stream.next_char() == Ch('\n') {
                // consume '\\n'
                self.consume_chars(2);
                continue;
            }

            // todo: move to its own util function (used for string & ident tokens)
            // TIMP: confirmation needed
            // according to css tests `-\\-` should parsed to `--`
            if self.stream.current_char() == Ch('\\')
                && !matches!(self.stream.next_char(), Ch(c) if c.is_ascii_hexdigit())
                && !matches!(self.stream.next_char(), Bytes::Eof)
            {
                // consume '\'
                self.stream.read_char();

                // consume char next to `\`
                value.push(self.stream.read_char().into());
                continue;
            }

            if self.is_start_of_escape(0) {
                value.push(self.consume_escaped_token());
                continue;
            }

            value.push(self.stream.read_char().into())
        }
    }

    /// 4.3.12. [Consume a number](https://www.w3.org/TR/css-syntax-3/#consume-number)
    ///
    /// Note: for the sake of simplicity, we exclude the number type mentioned in the algorithm.
    pub fn consume_number(&mut self) -> Number {
        let mut value = String::new();
        let lookahead = self.stream.current_char();

        if matches!(lookahead, Ch('+' | '-')) {
            value.push(self.stream.read_char().into());
        }

        value.push_str(&self.consume_digits());

        if self.stream.current_char() == Ch('.')
            && matches!(self.stream.next_char(), Ch(c) if c.is_numeric())
        {
            value.push_str(&self.consume_chars(2));
        }

        // type should be "number"
        value.push_str(&self.consume_digits());

        // todo: move them to gobal constants
        // U+0045: LATIN CAPITAL LETTER E (E)
        // U+0065: LATIN SMALL LETTER E (e)
        if matches!(self.stream.current_char(), Ch('\u{0045}' | '\u{0065}')) {
            value.push(self.stream.read_char().into());

            if matches!(self.stream.current_char(), Ch('-' | '+')) {
                value.push(self.stream.read_char().into());
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

        if value == "url" && self.stream.current_char() == Ch('(') {
            // consume '('
            self.stream.read_char();
            self.consume_whitespace();

            if self.is_any_of(vec!['"', '\'']) {
                return Token::Function(value);
            }

            return self.consume_url();
        } else if self.stream.current_char() == Ch('(') {
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
            if self.stream.current_char() == Ch(')') {
                // consume ')'
                self.stream.read_char();
                break;
            }

            if self.stream.eof() {
                // parser error
                break;
            }

            if self.stream.current_char().is_whitespace() {
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

            url.push(self.stream.read_char().into());
        }

        Token::Url(url)
    }

    /// 4.3.14. [Consume the remnants of a bad url](https://www.w3.org/TR/css-syntax-3/#consume-remnants-of-bad-url)
    ///
    /// Used is to consume enough of the input stream to reach a recovery point where normal tokenizing can resume.
    fn consume_remnants_of_bad_url(&mut self) {
        loop {
            // recovery point
            if self.stream.current_char() == Ch(')') || self.stream.eof() {
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

        while matches!(self.stream.current_char(), Ch(c) if c.is_ascii_hexdigit())
            && value.len() <= 5
        {
            value.push(self.stream.read_char().into());
        }

        if self.stream.current_char().is_whitespace() {
            self.stream.read_char();
        }

        if value.is_empty() {
            return default_char;
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

        loop {
            // TIMP: confirmation needed
            // according to css tests `-\\-` should parsed to `--`
            if self.stream.current_char() == Ch('\\')
                && !matches!(self.stream.next_char(), Ch(c) if c.is_ascii_hexdigit())
                && !matches!(self.stream.next_char(), Bytes::Eof)
            {
                // consume '\'
                self.stream.read_char();

                // consume char next to `\`
                value.push(self.stream.read_char().into());
                continue;
            }

            if self.is_start_of_escape(0) {
                value.push(self.consume_escaped_token());
                continue;
            }

            if !self.is_ident_char(self.stream.current_char().into()) {
                break;
            }

            value.push(self.stream.read_char().into());
        }

        value
    }

    pub fn consume_digits(&mut self) -> String {
        let mut value = String::new();

        while matches!(self.stream.current_char(), Ch(c) if c.is_numeric()) {
            value.push(self.stream.read_char().into())
        }

        value
    }

    pub fn consume_chars(&mut self, mut len: usize) -> String {
        let mut value = String::new();

        while len > 0 {
            value.push(self.stream.read_char().into());
            len -= 1
        }

        value
    }

    fn consume_whitespace(&mut self) -> Token {
        while self.stream.current_char().is_whitespace() {
            self.stream.read_char();
        }

        Token::Whitespace
    }

    /// [ident-start code point](https://www.w3.org/TR/css-syntax-3/#ident-start-code-point)
    fn is_ident_start(&self, char: char) -> bool {
        char.is_alphabetic() || !char.is_ascii() || char == '_'
    }

    /// [ident code point](https://www.w3.org/TR/css-syntax-3/#ident-start-code-point)
    fn is_ident_char(&self, char: char) -> bool {
        self.is_ident_start(char) || char.is_numeric() || char == '-'
    }

    /// def: [non-printable code point](https://www.w3.org/TR/css-syntax-3/#non-printable-code-point)
    fn is_non_printable_char(&self) -> bool {
        if let Ch(char) = self.stream.current_char() {
            (char >= get_unicode_char(UnicodeChar::Null)
                && char <= get_unicode_char(UnicodeChar::Backspace))
                || (char >= get_unicode_char(UnicodeChar::ShiftOut)
                    && char <= get_unicode_char(UnicodeChar::InformationSeparatorOne))
                || char == get_unicode_char(UnicodeChar::Tab)
                || char == get_unicode_char(UnicodeChar::Delete)
        } else {
            false
        }
    }

    /// 4.3.8. [Check if two code points are a valid escape](https://www.w3.org/TR/css-syntax-3/#starts-with-a-valid-escape)
    fn is_start_of_escape(&self, start: usize) -> bool {
        let current_char = self.stream.look_ahead(start);
        let next_char = self.stream.look_ahead(start + 1);

        current_char == Ch('\\') && next_char != Ch('\n')
    }

    /// [4.3.9. Check if three code points would start an ident sequence](https://www.w3.org/TR/css-syntax-3/#check-if-three-code-points-would-start-an-ident-sequence)
    fn is_next_3_points_starts_ident_seq(&self, start: usize) -> bool {
        let first = self.stream.look_ahead(start);
        let second = self.stream.look_ahead(start + 1);

        if first == Ch('-') {
            return self.is_ident_start(second.into())
                || second == Ch('-')
                || self.is_start_of_escape(start + 1);
        }

        if first == Ch('\\') {
            return self.is_start_of_escape(start);
        }

        self.is_ident_start(first.into())
    }

    fn is_signed_decimal(&self, start: usize) -> bool {
        let current = self.stream.look_ahead(start);
        let next = self.stream.look_ahead(start + 1);
        let last = self.stream.look_ahead(start + 2);

        // e.g. +1, -1, +.1, -0.01
        matches!(current, Ch('+' | '-'))
            && ((next == Ch('.') && last.is_numeric()) || next.is_numeric())
    }

    fn is_any_of(&self, chars: Vec<char>) -> bool {
        let current_char = self.stream.current_char();
        for char in chars {
            if current_char == Ch(char) {
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
            ("_123\\ident", "_123ident"),
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
            ("-rgba(", Token::Function("-rgba".into())),
            ("--rgba(", Token::Function("--rgba".into())),
            ("-\\26 -rgba(", Token::Function("-&-rgba".into())),
            ("0rgba()", Token::Function("0rgba".into())),
            ("-0rgba()", Token::Function("-0rgba".into())),
            ("_rgba()", Token::Function("_rgba".into())),
            ("rgbâ()", Token::Function("rgbâ".into())),
            ("\\30rgba()", Token::Function("0rgba".into())),
            ("rgba ()", Token::Ident("rgba".into())),
            ("-\\-rgba(", Token::Function("--rgba".into())),
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

    #[test]
    fn produce_stream_of_double_quoted_strings() {
        let mut is = InputStream::new();

        is.read_from_str(
            "\"\" \"Lorem 'îpsum'\" \"a\\\nb\" \"a\nb \"eof",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            // `\"\"`
            Token::QuotedString(String::new()),
            Token::Whitespace,
            // \"Lorem 'îpsum'\"
            Token::QuotedString("Lorem 'îpsum'".into()),
            Token::Whitespace,
            // `\"a\\\nb\"`
            Token::QuotedString("ab".into()),
            Token::Whitespace,
            Token::BadString("a".into()),
            Token::Whitespace,
            Token::Ident("b".into()),
            Token::Whitespace,
            Token::QuotedString("eof".into()),
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn procude_stream_of_single_quoted_strings() {
        let mut is = InputStream::new();

        is.read_from_str(
            "'' 'Lorem \"îpsum\"' 'a\\\nb' 'a\nb 'eof",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            // `\"\"`
            Token::QuotedString(String::new()),
            Token::Whitespace,
            // \"Lorem 'îpsum'\"
            Token::QuotedString("Lorem \"îpsum\"".into()),
            Token::Whitespace,
            // `\"a\\\nb\"`
            Token::QuotedString("ab".into()),
            Token::Whitespace,
            Token::BadString("a".into()),
            Token::Whitespace,
            Token::Ident("b".into()),
            Token::Whitespace,
            Token::QuotedString("eof".into()),
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_urls_with_strings() {
        let mut is = InputStream::new();

        is.read_from_str(
            "url( '') url('Lorem \"îpsum\"'\n) url('a\\\nb' ) url('a\nb) url('eof",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            // `url( '')`
            Token::Function("url".into()),
            Token::QuotedString("".into()),
            Token::RParen,
            Token::Whitespace,
            // `url('Lorem \"îpsum\"'\n)`
            Token::Function("url".into()),
            Token::QuotedString("Lorem \"îpsum\"".into()),
            Token::Whitespace,
            Token::RParen,
            Token::Whitespace,
            // `url('a\\\nb' )`
            Token::Function("url".into()),
            Token::QuotedString("ab".into()),
            Token::Whitespace,
            Token::RParen,
            Token::Whitespace,
            // `url('a\nb)`
            Token::Function("url".into()),
            Token::BadString("a".into()),
            Token::Whitespace,
            Token::Ident("b".into()),
            Token::RParen,
            Token::Whitespace,
            // `url('eof`
            Token::Function("url".into()),
            Token::QuotedString("eof".into()),
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn produce_valid_stream_of_css_tokens() {
        let mut is = InputStream::new();

        is.read_from_str(
            "
        /* Navbar */
        #header .nav {
            font-size: 1.1rem;
        }

        @media screen (max-width: 200px) {}

        content: \"me \\26  you\";

        background: url(https://gosub.io);
        ",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            // 1st css rule
            Token::Whitespace,
            Token::IDHash("header".into()),
            Token::Whitespace,
            Token::Delim('.'),
            Token::Ident("nav".into()),
            Token::Whitespace,
            Token::LCurly,
            Token::Whitespace,
            Token::Ident("font-size".into()),
            Token::Colon,
            Token::Whitespace,
            Token::Dimension {
                unit: "rem".into(),
                value: 1.1,
            },
            Token::Semicolon,
            Token::Whitespace,
            Token::RCurly,
            Token::Whitespace,
            // 2nd css rule (AtRule)
            Token::AtKeyword("media".into()),
            Token::Whitespace,
            Token::Ident("screen".into()),
            Token::Whitespace,
            Token::LParen,
            Token::Ident("max-width".into()),
            Token::Colon,
            Token::Whitespace,
            Token::Dimension {
                unit: "px".into(),
                value: 200.0,
            },
            Token::RParen,
            Token::Whitespace,
            Token::LCurly,
            Token::RCurly,
            Token::Whitespace,
            // 3rd css declaration
            Token::Ident("content".into()),
            Token::Colon,
            Token::Whitespace,
            Token::QuotedString("me & you".into()),
            Token::Semicolon,
            Token::Whitespace,
            // 4th css declaration
            Token::Ident("background".into()),
            Token::Colon,
            Token::Whitespace,
            Token::Url("https://gosub.io".into()),
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        tokenizer.consume_whitespace();
        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }
    }

    #[test]
    fn parse_rgba_expr() {
        let mut is = InputStream::new();

        is.read_from_str(
            "
            rgba(255, 50%, 0%, 1)
        ",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            Token::Whitespace,
            Token::Function("rgba".into()),
            Token::Number(255.0),
            Token::Comma,
            Token::Whitespace,
            Token::Percentage(50.0),
            Token::Comma,
            Token::Whitespace,
            Token::Percentage(0.0),
            Token::Comma,
            Token::Whitespace,
            Token::Number(1.0),
            Token::RParen,
            Token::Whitespace,
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }
    }

    #[test]
    fn parse_cdo_and_cdc() {
        let mut is = InputStream::new();

        is.read_from_str(
            "/* CDO/CDC are not special */ <!-- --> {}",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            Token::Whitespace,
            Token::CDO,
            Token::Whitespace,
            Token::CDC,
            Token::Whitespace,
            Token::LCurly,
            Token::RCurly,
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }
    }

    #[test]
    fn parse_spaced_comments() {
        let mut is = InputStream::new();

        is.read_from_str("/*/*///** /* **/*//* ", Some(Encoding::UTF8));

        let tokens = vec![
            Token::Delim('/'),
            Token::Delim('*'),
            Token::Delim('/'),
            Token::EOF,
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_all_whitespaces() {
        let mut is = InputStream::new();

        is.read_from_str("  \t\t\r\n\nRed ", Some(Encoding::UTF8));

        let tokens = vec![
            Token::Whitespace,
            Token::Ident("Red".into()),
            Token::Whitespace,
            Token::EOF,
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_at_keywords() {
        let mut is = InputStream::new();

        is.read_from_str(
            "@media0 @-Media @--media @0media @-0media @_media @.media @medİa @\\30 media\\",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            Token::AtKeyword("media0".into()),
            Token::Whitespace,
            Token::AtKeyword("-Media".into()),
            Token::Whitespace,
            Token::AtKeyword("--media".into()),
            Token::Whitespace,
            // `@0media` => [@, 0, meida]
            Token::Delim('@'),
            Token::Dimension {
                unit: "media".into(),
                value: 0.0,
            },
            Token::Whitespace,
            // `@-0media` => [@, -0, meida]
            Token::Delim('@'),
            Token::Dimension {
                unit: "media".into(),
                value: -0.0,
            },
            Token::Whitespace,
            // `@_media`
            Token::AtKeyword("_media".into()),
            Token::Whitespace,
            // `@.meida` => [@, ., media]
            Token::Delim('@'),
            Token::Delim('.'),
            Token::Ident("media".into()),
            Token::Whitespace,
            // `@medİa`
            Token::AtKeyword("medİa".into()),
            Token::Whitespace,
            // `@\\30 media`
            Token::AtKeyword("0media\u{FFFD}".into()),
            Token::EOF,
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_id_selectors() {
        let mut is = InputStream::new();

        is.read_from_str(
            "#red0 #-Red #--red #-\\-red #0red #-0red #_Red #.red #rêd #êrd #\\.red\\",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            Token::IDHash("red0".into()),
            Token::Whitespace,
            Token::IDHash("-Red".into()),
            Token::Whitespace,
            Token::IDHash("--red".into()),
            Token::Whitespace,
            // `#--\\red`
            Token::IDHash("--red".into()),
            Token::Whitespace,
            // `#0red` => 0red
            Token::Hash("0red".into()),
            Token::Whitespace,
            // `#-0red`
            Token::Hash("-0red".into()),
            Token::Whitespace,
            // `#_Red`
            Token::IDHash("_Red".into()),
            Token::Whitespace,
            // `#.red` => [#, ., red]
            Token::Delim('#'),
            Token::Delim('.'),
            Token::Ident("red".into()),
            Token::Whitespace,
            // `#rêd`
            Token::IDHash("rêd".into()),
            Token::Whitespace,
            // `#êrd`
            Token::IDHash("êrd".into()),
            Token::Whitespace,
            // `#\\.red\\`
            Token::IDHash(".red\u{FFFD}".into()),
            Token::EOF,
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_dimension_tokens() {
        let mut is = InputStream::new();

        is.read_from_str(
            "12red0 12.0-red 12--red 12-\\-red 120red 12-0red 12\\0000red 12_Red 12.red 12rêd",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            // `12red0`
            Token::Dimension {
                unit: "red0".into(),
                value: 12.0,
            },
            Token::Whitespace,
            // `12.0-red`
            Token::Dimension {
                unit: "-red".into(),
                value: 12.0,
            },
            Token::Whitespace,
            // `12--red`
            Token::Dimension {
                unit: "--red".into(),
                value: 12.0,
            },
            Token::Whitespace,
            // `12-\\-red`
            Token::Dimension {
                unit: "--red".into(),
                value: 12.0,
            },
            Token::Whitespace,
            // `120red`
            Token::Dimension {
                unit: "red".into(),
                value: 120.0,
            },
            Token::Whitespace,
            // `12-0red` => [12, -0red]
            Token::Number(12.0),
            Token::Dimension {
                unit: "red".into(),
                value: -0.0,
            },
            Token::Whitespace,
            // `12\u{0000}red`
            Token::Dimension {
                unit: "\u{FFFD}red".into(),
                value: 12.0,
            },
            Token::Whitespace,
            // `12_Red`
            Token::Dimension {
                unit: "_Red".into(),
                value: 12.0,
            },
            Token::Whitespace,
            // `12.red` => [12, ., red]
            Token::Number(12.0),
            Token::Delim('.'),
            Token::Ident("red".into()),
            Token::Whitespace,
            // `12rêd`
            Token::Dimension {
                unit: "rêd".into(),
                value: 12.0,
            },
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_dimension_tokens_2() {
        let mut is = InputStream::new();

        is.read_from_str(
            "12e2px +34e+1px -45E-0px .68e+3px +.79e-1px -.01E2px 2.3E+1px +45.0e6px -0.67e0px",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            // `12e2px`
            Token::Dimension {
                unit: "px".into(),
                value: 1200.0,
            },
            Token::Whitespace,
            // `+34e+1px`
            Token::Dimension {
                unit: "px".into(),
                value: 340.0,
            },
            Token::Whitespace,
            // `-45E-0px`
            Token::Dimension {
                unit: "px".into(),
                value: -45.0,
            },
            Token::Whitespace,
            // `.68e+3px`
            Token::Dimension {
                unit: "px".into(),
                value: 680.0,
            },
            Token::Whitespace,
            // `+.79e-1px`
            Token::Dimension {
                unit: "px".into(),
                value: 0.079,
            },
            Token::Whitespace,
            // `-.01E2px`
            Token::Dimension {
                unit: "px".into(),
                value: -1.0,
            },
            Token::Whitespace,
            // `2.3E+1px`
            Token::Dimension {
                unit: "px".into(),
                value: 23.0,
            },
            Token::Whitespace,
            // `+45.0e6px`
            Token::Dimension {
                unit: "px".into(),
                value: 45000000.0,
            },
            Token::Whitespace,
            // `-0.67e0px`
            Token::Dimension {
                unit: "px".into(),
                value: -0.67,
            },
            Token::EOF,
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_percentage() {
        let mut is = InputStream::new();

        is.read_from_str(
            "12e2% +34e+1% -45E-0% .68e+3% +.79e-1% -.01E2% 2.3E+1% +45.0e6% -0.67e0%",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            // `12e2%`
            Token::Percentage(1200.0),
            Token::Whitespace,
            // `+34e+1%`
            Token::Percentage(340.0),
            Token::Whitespace,
            // `-45E-0%`
            Token::Percentage(-45.0),
            Token::Whitespace,
            // `.68e+3%`
            Token::Percentage(680.0),
            Token::Whitespace,
            // `+.79e-1%`
            Token::Percentage(0.079),
            Token::Whitespace,
            // `-.01E2%`
            Token::Percentage(-1.0),
            Token::Whitespace,
            // `2.3E+1%`
            Token::Percentage(23.0),
            Token::Whitespace,
            // `+45.0e6%`
            Token::Percentage(45000000.0),
            Token::Whitespace,
            // `-0.67e0%`
            Token::Percentage(-0.67),
            Token::EOF,
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_css_seq_1() {
        let mut is = InputStream::new();

        is.read_from_str(
            "a:not([href^=http\\:],  [href ^=\t'https\\:'\n]) { color: rgba(0%, 100%, 50%); }",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            Token::Ident("a".into()),
            Token::Colon,
            Token::Function("not".into()),
            Token::LBracket,
            Token::Ident("href".into()),
            Token::Delim('^'),
            Token::Delim('='),
            Token::Ident("http:".into()),
            Token::RBracket,
            Token::Comma,
            Token::Whitespace,
            Token::LBracket,
            Token::Ident("href".into()),
            Token::Whitespace,
            Token::Delim('^'),
            Token::Delim('='),
            Token::Whitespace,
            Token::QuotedString("https:".into()),
            Token::Whitespace,
            Token::RBracket,
            Token::RParen,
            Token::Whitespace,
            Token::LCurly,
            Token::Whitespace,
            Token::Ident("color".into()),
            Token::Colon,
            Token::Whitespace,
            Token::Function("rgba".into()),
            Token::Percentage(0.0),
            Token::Comma,
            Token::Whitespace,
            Token::Percentage(100.0),
            Token::Comma,
            Token::Whitespace,
            Token::Percentage(50.0),
            Token::RParen,
            Token::Semicolon,
            Token::Whitespace,
            Token::RCurly,
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_css_seq_2() {
        let mut is = InputStream::new();

        is.read_from_str("red-->/* Not CDC */", Some(Encoding::UTF8));

        let tokens = vec![Token::Ident("red--".into()), Token::Delim('>'), Token::EOF];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_css_seq_3() {
        let mut is = InputStream::new();

        is.read_from_str("\\- red0 -red --red -\\-red\\ blue 0red -0red \\0000red _Red .red rêd r\\êd \\007F\\0080\\0081", Some(Encoding::UTF8));

        let tokens = vec![
            // `\\-`
            Token::Ident("-".into()),
            Token::Whitespace,
            // `red0`
            Token::Ident("red0".into()),
            Token::Whitespace,
            // `-red`
            Token::Ident("-red".into()),
            Token::Whitespace,
            // `--red`
            Token::Ident("--red".into()),
            Token::Whitespace,
            // `-\\-red\\ blue`
            Token::Ident("--red blue".into()),
            Token::Whitespace,
            // `0red`
            Token::Dimension {
                unit: "red".into(),
                value: 0.0,
            },
            Token::Whitespace,
            // `-0red`
            Token::Dimension {
                unit: "red".into(),
                value: -0.0,
            },
            Token::Whitespace,
            // `\\0000red`
            Token::Ident("\u{FFFD}red".into()),
            Token::Whitespace,
            // `_Red`
            Token::Ident("_Red".into()),
            Token::Whitespace,
            // `.red` => [., red]
            Token::Delim('.'),
            Token::Ident("red".into()),
            Token::Whitespace,
            // `rêd`
            Token::Ident("rêd".into()),
            Token::Whitespace,
            // `r\\êd`
            Token::Ident("rêd".into()),
            Token::Whitespace,
            // `\\007F\\0080\\0081`
            Token::Ident("\u{7f}\u{80}\u{81}".into()),
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_css_seq_4() {
        let mut is = InputStream::new();

        is.read_from_str(
            "p[example=\"\\\nfoo(int x) {\\\n   this.x = x;\\\n}\\\n\"]",
            Some(Encoding::UTF8),
        );

        let tokens = vec![
            Token::Ident("p".into()),
            Token::LBracket,
            Token::Ident("example".into()),
            Token::Delim('='),
            Token::QuotedString("foo(int x) {   this.x = x;}".into()),
            Token::RBracket,
        ];
        let mut tokenizer = Tokenizer::new(&mut is);

        for token in tokens {
            assert_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn consume_tokenizer_as_stream_of_tokens() {
        let mut is = InputStream::new();
        is.read_from_str("[][]", Some(Encoding::UTF8));

        let mut tokenizer = Tokenizer::new(&mut is);
        tokenizer.consume_all();

        assert_eq!(tokenizer.lookahead(0), Token::LBracket);
        assert_eq!(tokenizer.lookahead(1), Token::RBracket);
        assert_eq!(tokenizer.lookahead(4), Token::EOF);

        assert_eq!(tokenizer.consume(), Token::LBracket);
        assert_eq!(tokenizer.lookahead(0), Token::RBracket);
    }
}
