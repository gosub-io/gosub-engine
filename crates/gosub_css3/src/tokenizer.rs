use crate::location::Location;
use crate::unicode::{get_unicode_char, UnicodeChar};
use gosub_shared::byte_stream::Character::Ch;
use gosub_shared::byte_stream::{ByteStream, Character, Stream};
use std::fmt;

pub type Number = f32;

#[derive(Debug, PartialEq, Clone)]
pub enum TokenType {
    /// A [`<at-keyword-token>`](https://drafts.csswg.org/css-syntax/#at-keyword-token-diagram)
    ///
    /// The value does not include the `@` marker.
    AtKeyword(String),
    Ident(String),
    Function(String),
    Url(String),
    BadUrl(String),
    Dimension {
        value: Number,
        unit: String,
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
    /// A `<{-token>`
    LCurly,
    /// A `<}-token>`
    RCurly,
    /// A `<(-token>`
    LParen,
    /// A `<)-token>`
    RParen,
    /// A `<[-token>`
    LBracket,
    /// A `<]-token>`
    RBracket,
    /// A `<comma-token>`
    Comma,
    /// A `:` `<colon-token>`
    Colon,
    /// A `;` `<semicolon-token>`
    Semicolon,
    // A `<!--` `<CDO-token>`
    Cdo,
    // A `-->` `<CDC-token>`
    Cdc,
    // // A `<EOF-token>`
    Eof,
    // A comment
    Comment(String),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Token {
    /// Type of the token
    pub token_type: TokenType,
    /// Location of the token in the stream
    pub location: Location,
}

impl Token {
    /// Returns a new token for the given type on the given location
    fn new(token_type: TokenType, location: Location) -> Token {
        Token {
            token_type,
            location,
        }
    }

    fn new_delim(c: char, location: Location) -> Token {
        Token::new(TokenType::Delim(c), location)
    }

    fn new_id_hash(value: &str, location: Location) -> Token {
        Token::new(TokenType::IDHash(value.to_string()), location)
    }

    fn new_hash(value: &str, location: Location) -> Token {
        Token::new(TokenType::Hash(value.to_string()), location)
    }

    fn new_atkeyword(keyword: &str, location: Location) -> Token {
        Token::new(TokenType::AtKeyword(keyword.to_string()), location)
    }

    fn new_number(value: Number, location: Location) -> Token {
        Token::new(TokenType::Number(value), location)
    }

    fn new_percentage(value: Number, location: Location) -> Token {
        Token::new(TokenType::Percentage(value), location)
    }

    fn new_dimension(value: Number, unit: &str, location: Location) -> Token {
        Token::new(
            TokenType::Dimension {
                value,
                unit: unit.to_string(),
            },
            location,
        )
    }

    fn new_ident(value: &str, location: Location) -> Token {
        Token::new(TokenType::Ident(value.to_string()), location)
    }

    fn new_function(value: &str, location: Location) -> Token {
        Token::new(TokenType::Function(value.to_string()), location)
    }

    fn new_quoted_string(value: &str, location: Location) -> Token {
        Token::new(TokenType::QuotedString(value.to_string()), location)
    }

    fn new_bad_string(value: &str, location: Location) -> Token {
        Token::new(TokenType::BadString(value.to_string()), location)
    }

    fn new_url(value: &str, location: Location) -> Token {
        Token::new(TokenType::Url(value.to_string()), location)
    }

    fn new_bad_url(value: &str, location: Location) -> Token {
        Token::new(TokenType::BadUrl(value.to_string()), location)
    }
}

impl Token {
    pub(crate) fn is_comma(&self) -> bool {
        matches!(self.token_type, TokenType::Comma)
    }

    pub(crate) fn is_string(&self) -> bool {
        matches!(self.token_type, TokenType::QuotedString(_))
    }

    pub(crate) fn is_ident(&self) -> bool {
        matches!(self.token_type, TokenType::Ident(_))
    }

    #[allow(dead_code)]
    pub(crate) fn is_comment(&self) -> bool {
        matches!(self.token_type, TokenType::Comment(_))
    }

    #[allow(dead_code)]
    pub(crate) fn is_whitespace(&self) -> bool {
        matches!(self.token_type, TokenType::Whitespace)
    }

    pub(crate) fn is_colon(&self) -> bool {
        matches!(self.token_type, TokenType::Colon)
    }

    pub(crate) fn is_delim(&self, delim: char) -> bool {
        matches!(self.token_type, TokenType::Delim(c) if c == delim)
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let string = match self.token_type.clone() {
            TokenType::AtKeyword(val)
            | TokenType::Url(val)
            | TokenType::Comment(val)
            | TokenType::BadUrl(val)
            | TokenType::Hash(val)
            | TokenType::IDHash(val)
            | TokenType::Ident(val)
            | TokenType::Function(val)
            | TokenType::QuotedString(val)
            | TokenType::BadString(val) => val,
            TokenType::Delim(val) => val.to_string(),
            TokenType::Number(val) => val.to_string(),
            TokenType::Percentage(val) => format!("{}%", val),
            TokenType::Dimension { unit, value } => format!("{}{}", value, unit),
            TokenType::Cdc => "-->".into(),
            TokenType::Cdo => "<!--".into(),
            TokenType::Colon => ":".into(),
            TokenType::Semicolon => ";".into(),
            TokenType::Comma => ",".into(),
            TokenType::LBracket => "[".into(),
            TokenType::RBracket => "]".into(),
            TokenType::LCurly => "{".into(),
            TokenType::RCurly => "}".into(),
            TokenType::LParen => "(".into(),
            TokenType::RParen => ")".into(),
            TokenType::Whitespace => " ".into(),
            TokenType::Eof => "eof".into(),
        };

        write!(f, "{string}")
    }
}

/// CSS Tokenizer according to the [w3 specification](https://www.w3.org/TR/css-syntax-3/#tokenization)
#[allow(dead_code)]
pub struct Tokenizer<'stream> {
    stream: &'stream mut ByteStream,
    /// Position on the NEXT read to consume. If it's outside the vec list, it will return EOF
    position: usize,
    /// Full list of all tokens produced by the tokenizer
    tokens: Vec<Token>,
    /// List of all line endings
    line_endings: Vec<usize>,
    /// Start position of the stream (this does not have to be 1/1)
    start_location: Location,
    /// Current position of the stream, to get the absolute position, we must add start_location to it
    cur_location: Location,
    /// WHen true, the stream is closed and no more tokens can be produced
    eof: bool,
}

impl<'stream> Tokenizer<'stream> {
    /// Creates a new tokenizer with the given stream that starts on the given location. This does not have
    /// to be 1/1, but can be any location.
    pub fn new(stream: &'stream mut ByteStream, location: Location) -> Self {
        Self {
            stream,
            position: 0,
            tokens: Vec::new(),
            start_location: location.clone(),
            cur_location: Location::new(1, 1, 0),
            eof: false,
            line_endings: Vec::new(),
        }
    }

    /// Returns the current location and takes the start location into account
    pub fn current_location(&self) -> Location {
        Location::new(
            self.start_location.line() + self.cur_location.line() - 1,
            self.start_location.column() + self.cur_location.column() - 1,
            self.cur_location.offset(),
        )
    }

    /// Returns true when there is no next element, and the stream is closed
    pub fn eof(&self) -> bool {
        self.stream.eof() && self.position >= self.tokens.len()
    }

    /// Returns the current token. This can be either EOF at the end of the stream, of EOF when we
    /// haven't read anything. It would be more correct to return this in an Option.
    pub fn current(&self) -> Token {
        if self.position == 0 {
            // We haven't read anything yet. We can't really return anything (we haven't read anything), so we return EOF
            return Token::new(TokenType::Eof, self.current_location().clone());
        }
        if self.position > self.tokens.len() {
            return Token::new(TokenType::Eof, self.current_location().clone());
        }

        self.tokens[self.position - 1].clone()
    }

    /// Looks ahead at the next NON-WHITESPACE AND NON-COMMENT token.
    pub(crate) fn lookahead_sc(&mut self, offset: usize) -> Token {
        let mut i = offset;

        loop {
            let t = self.lookahead(i);
            match t.token_type {
                TokenType::Whitespace | TokenType::Comment(_) => {
                    i += 1;
                }
                _ => return t,
            }
        }
    }

    /// Looks ahead at the next token with offset. So lookahead(1) will look at the next character
    /// that will be consumed with consume()
    pub fn lookahead(&mut self, offset: usize) -> Token {
        while (self.tokens.len() - 1) < (self.position + offset) {
            let token = self.consume_token();
            self.tokens.push(token);
        }

        let pos: isize = (self.position + offset) as isize;
        if pos < 0 || pos >= self.tokens.len() as isize {
            // Both start of the stream, and end of the stream return EOF
            return Token::new(TokenType::Eof, self.current_location().clone());
        }

        self.tokens[pos as usize].clone()
    }

    /// Consumes the next token and returns it
    pub fn consume(&mut self) -> Token {
        if self.tokens.is_empty() || self.tokens.len() == self.position {
            let token = self.consume_token();
            self.tokens.push(token);
        }

        let token = &self.tokens[self.position];
        self.position += 1;

        log::trace!("{:?}", token);

        token.clone()
    }

    /// Reconsumes will push the current position back so the next read will be the same token
    pub fn reconsume(&mut self) {
        if self.position > 0 {
            self.position -= 1;
            self.cur_location = self.tokens[self.position].location.clone();
        }
    }

    #[cfg(test)]
    fn consume_all(&mut self) {
        while !self.stream.eof() {
            let token = self.consume_token();
            self.tokens.push(token);
        }

        self.position = 0;
    }

    /// 4.3.1. [Consume a token](https://www.w3.org/TR/css-syntax-3/#consume-token)
    fn consume_token(&mut self) -> Token {
        while self.look_ahead_slice(2) == "/*" {
            self.consume_comment();
        }

        // todo: reframe the concept of "tokenizer::current" and "is::current" and "is::next"
        let current = self.current_char();
        let loc = self.current_location().clone();

        let t = match current {
            Character::Surrogate(_) => {
                self.next_char();
                // @todo: we found a surrogate. Just return a replacement char
                Token::new(TokenType::Delim('\u{FFFD}'), loc)
            }
            Character::StreamEnd => Token::new(TokenType::Eof, loc),
            Character::StreamEmpty => {
                // @todo: we are in a situation where we don't have more characters yet, but the stream is still open. We should wait
                // for more characters to come in.
                Token::new(TokenType::Eof, loc)
            }
            Ch(c) if c.is_whitespace() => {
                self.consume_whitespace();
                Token::new(TokenType::Whitespace, loc)
            }
            // note: consume_string_token doesn't work as expected
            Ch('"' | '\'') => self.consume_string_token(),
            Ch(c @ '#') => {
                // consume '#'
                self.next_char();

                if self.is_ident_char(self.current_char().into()) || self.is_start_of_escape(0) {
                    return if self.is_next_3_points_starts_ident_seq(0) {
                        Token::new_id_hash(self.consume_ident().as_str(), loc)
                    } else {
                        Token::new_hash(self.consume_ident().as_str(), loc)
                    };
                }

                Token::new_delim(c, loc)
            }
            Ch(')') => {
                self.next_char();
                Token::new(TokenType::RParen, loc)
            }
            Ch('(') => {
                self.next_char();
                Token::new(TokenType::LParen, loc)
            }
            Ch('[') => {
                self.next_char();
                Token::new(TokenType::LBracket, loc)
            }
            Ch(']') => {
                self.next_char();
                Token::new(TokenType::RBracket, loc)
            }
            Ch('{') => {
                self.next_char();
                Token::new(TokenType::LCurly, loc)
            }
            Ch('}') => {
                self.next_char();
                Token::new(TokenType::RCurly, loc)
            }
            Ch(',') => {
                self.next_char();
                Token::new(TokenType::Comma, loc)
            }
            Ch(':') => {
                self.next_char();
                Token::new(TokenType::Colon, loc)
            }
            Ch(';') => {
                self.next_char();
                Token::new(TokenType::Semicolon, loc)
            }
            Ch(c @ '+') => {
                if self.is_signed_decimal(0) {
                    return self.consume_numeric_token();
                }

                // consume '+'
                self.next_char();
                Token::new_delim(c, loc)
            }
            Ch('.') => {
                if matches!(self.stream.look_ahead(1), Ch(c) if c.is_numeric()) {
                    return self.consume_numeric_token();
                }

                // consume '.'
                self.next_char();
                Token::new_delim('.', loc)
            }
            Ch(c @ '-') => {
                if self.is_signed_decimal(0) {
                    return self.consume_numeric_token();
                }

                let cdc_token = "-->";
                if self.look_ahead_slice(cdc_token.len()) == cdc_token {
                    // consume '--'
                    self.consume_chars(cdc_token.len());
                    return Token::new(TokenType::Cdc, loc);
                }

                if self.is_next_3_points_starts_ident_seq(0) {
                    return self.consume_ident_like_seq();
                }

                // consume '-'
                self.next_char();
                Token::new_delim(c, loc)
            }
            Ch(c @ '<') => {
                let cdo_token = "<!--";
                if self.look_ahead_slice(cdo_token.len()) == cdo_token {
                    // consume "<!--"
                    self.consume_chars(cdo_token.len());
                    return Token::new(TokenType::Cdo, loc);
                }

                // consume '<'
                self.next_char();
                Token::new_delim(c, loc)
            }
            Ch(c @ '@') => {
                // consume '@'
                self.next_char();

                if self.is_next_3_points_starts_ident_seq(0) {
                    return Token::new_atkeyword(self.consume_ident().as_str(), loc);
                }

                Token::new_delim(c, loc)
            }
            Ch(c @ '\\') => {
                if self.is_start_of_escape(0) {
                    return self.consume_ident_like_seq();
                }

                // parser error
                // consume '\'
                self.next_char();
                Token::new_delim(c, loc)
            }
            Ch(c) if c.is_numeric() => self.consume_numeric_token(),
            Ch(c) if self.is_ident_start(c) => self.consume_ident_like_seq(),
            Ch(c) => {
                self.next_char();
                Token::new(TokenType::Delim(c), loc)
            }
        };

        t
    }

    /// 4.3.2. [Consume comments](https://www.w3.org/TR/css-syntax-3/#consume-comment)
    fn consume_comment(&mut self) -> String {
        let mut comment = String::new();
        if self.look_ahead_slice(2) == "/*" {
            // consume '/*'
            comment.push_str(&self.consume_chars(2));

            while self.look_ahead_slice(2) != "*/" && !self.stream.eof() {
                comment.push(self.next_char().into());
            }

            // consume '*/'
            comment.push_str(&self.consume_chars(2));
        };

        comment
    }

    /// 4.3.3. [Consume a numeric token]()
    /// Returns either a `<number-token>`, `<percentage-token>`, or `<dimension-token>`.
    fn consume_numeric_token(&mut self) -> Token {
        let number = self.consume_number();

        let loc = self.current_location().clone();

        if self.is_next_3_points_starts_ident_seq(0) {
            let unit = self.consume_ident();

            return Token::new_dimension(number, unit.as_str(), loc);
        } else if self.current_char() == Ch('%') {
            // consume '%'
            self.next_char();
            return Token::new_percentage(number, loc);
        }

        Token::new_number(number, loc)
    }

    /// 4.3.5. [Consume a string token](https://www.w3.org/TR/css-syntax-3/#consume-string-token)
    ///
    /// Returns either a `<string-token>` or `<bad-string-token>`.
    fn consume_string_token(&mut self) -> Token {
        let loc = self.current_location().clone();

        // consume string starting: (') or (") ...
        let ending = self.next_char();
        let mut value = String::new();

        loop {
            // if eof => parser error => return the current string
            if self.current_char() == ending || self.stream.eof() {
                // consume string ending
                self.next_char();
                return Token::new_quoted_string(value.as_str(), loc);
            }

            // newline: parser error
            if self.current_char() == Ch('\n') {
                // note: don't consume '\n'
                return Token::new_bad_string(value.as_str(), loc);
            }

            if self.current_char() == Ch('\\') && self.stream.look_ahead(1) == Ch('\n') {
                // consume '\\n'
                self.consume_chars(2);
                continue;
            }

            // todo: move to its own util function (used for string & ident tokens)
            // TIMP: confirmation needed
            // according to css tests `-\\-` should parsed to `--`
            if self.current_char() == Ch('\\')
                && !matches!(self.stream.look_ahead(1), Ch(c) if c.is_ascii_hexdigit())
                && !matches!(self.stream.look_ahead(1), Character::StreamEnd)
            {
                // consume '\'
                self.next_char();

                // consume char next to `\`
                value.push(self.next_char().into());
                continue;
            }

            if self.is_start_of_escape(0) {
                value.push(self.consume_escaped_token());
                continue;
            }

            value.push(self.next_char().into());
        }
    }

    /// 4.3.12. [Consume a number](https://www.w3.org/TR/css-syntax-3/#consume-number)
    ///
    /// Note: for the sake of simplicity, we exclude the number type mentioned in the algorithm.
    fn consume_number(&mut self) -> Number {
        let mut value = String::new();
        let lookahead = self.current_char();

        if matches!(lookahead, Ch('+' | '-')) {
            value.push(self.next_char().into());
        }

        value.push_str(&self.consume_digits());

        if self.current_char() == Ch('.')
            && matches!(self.stream.look_ahead(1), Ch(c) if c.is_numeric())
        {
            value.push_str(&self.consume_chars(2));

            // type should be "number"
            value.push_str(&self.consume_digits());
        }

        // todo: move them to global constants
        // U+0045: LATIN CAPITAL LETTER E (E)
        // U+0065: LATIN SMALL LETTER E (e)
        let c1 = self.stream.look_ahead(0);
        let c2 = self.stream.look_ahead(1);
        let c3 = self.stream.look_ahead(2);
        if (c1 == Ch('\u{0045}') || c1 == Ch('\u{0065}'))
            && (((c2 == Ch('-') || c2 == Ch('+')) && c3.is_numeric()) || c2.is_numeric())
        {
            value.push(self.next_char().into());
            value.push(self.next_char().into());
            value.push_str(&self.consume_digits());
        }

        value.parse().expect("failed to parse number")
    }

    /// 4.3.4. [Consume an ident-like token](https://www.w3.org/TR/css-syntax-3/#consume-ident-like-token)
    ///
    /// Returns: `<ident-token>`, `<function-token>`, `<url-token>`, or `<bad-url-token>`.
    fn consume_ident_like_seq(&mut self) -> Token {
        let loc = self.current_location().clone();

        let value = self.consume_ident();

        if value == "url" && self.current_char() == Ch('(') {
            // consume '('
            self.next_char();
            self.consume_whitespace();

            if self.is_any_of(vec!['"', '\'']) {
                return Token::new_function(value.as_str(), loc);
            }

            return self.consume_url();
        } else if self.current_char() == Ch('(') {
            // consume '('
            self.next_char();
            return Token::new_function(value.as_str(), loc);
        }

        return Token::new_ident(value.as_str(), loc);
    }

    /// 4.3.6. [Consume a url token](https://www.w3.org/TR/css-syntax-3/#consume-a-url-token)
    ///
    /// Returns either a `<url-token>` or a `<bad-url-token>`
    fn consume_url(&mut self) -> Token {
        let mut url = String::new();

        let loc = self.current_location().clone();

        self.consume_whitespace();

        loop {
            if self.current_char() == Ch(')') {
                // consume ')'
                self.next_char();
                break;
            }

            if self.stream.eof() {
                // parser error
                break;
            }

            if self.current_char().is_whitespace() {
                self.consume_whitespace();
                continue;
            }

            if self.is_any_of(vec!['"', '\'', '(']) || self.is_non_printable_char() {
                // parse error
                self.consume_remnants_of_bad_url();
                return Token::new_bad_url(url.as_str(), loc);
            }

            if self.is_start_of_escape(0) {
                url.push(self.consume_escaped_token());
                continue;
            }

            url.push(self.next_char().into());
        }

        return Token::new_url(url.as_str(), loc);
    }

    /// 4.3.14. [Consume the remnants of a bad url](https://www.w3.org/TR/css-syntax-3/#consume-remnants-of-bad-url)
    ///
    /// Used is to consume enough of the input stream to reach a recovery point where normal tokenizing can resume.
    fn consume_remnants_of_bad_url(&mut self) {
        loop {
            // recovery point
            if self.current_char() == Ch(')') || self.stream.eof() {
                break;
            }

            if self.is_start_of_escape(0) {
                self.consume_escaped_token();
            }

            // todo: parse escaped code point.
            self.next_char();
        }
    }

    /// 4.3.7. [Consume an escaped code point](https://www.w3.org/TR/css-syntax-3/#consume-an-escaped-code-point)
    fn consume_escaped_token(&mut self) -> char {
        // consume '\'
        self.next_char();

        let mut value = String::new();

        let default_char = get_unicode_char(&UnicodeChar::ReplacementCharacter);
        // eof: parser error
        if self.stream.eof() {
            return default_char;
        }

        while matches!(self.current_char(), Ch(c) if c.is_ascii_hexdigit()) && value.len() <= 5 {
            value.push(self.next_char().into());
        }

        if self.current_char().is_whitespace() {
            self.next_char();
        }

        if value.is_empty() {
            return default_char;
        }

        let as_u32 = u32::from_str_radix(&value, 16).expect("unable to parse hex string as number");

        // todo: look for better implementation
        if let Some(char) = char::from_u32(as_u32) {
            if char == get_unicode_char(&UnicodeChar::Null)
                || char >= get_unicode_char(&UnicodeChar::MaxAllowed)
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
    fn consume_ident(&mut self) -> String {
        let mut value = String::new();

        loop {
            // TIMP: confirmation needed
            // according to css tests `-\\-` should parsed to `--`
            if self.current_char() == Ch('\\')
                && !matches!(self.stream.look_ahead(1), Ch(c) if c.is_ascii_hexdigit())
                && !matches!(self.stream.look_ahead(1), Character::StreamEnd)
            {
                // consume '\'
                self.next_char();

                // consume char next to `\`
                value.push(self.next_char().into());
                continue;
            }

            if self.is_start_of_escape(0) {
                value.push(self.consume_escaped_token());
                continue;
            }

            if !self.is_ident_char(self.current_char().into()) {
                break;
            }

            value.push(self.next_char().into());
        }

        value
    }

    fn consume_digits(&mut self) -> String {
        let mut value = String::new();

        while matches!(self.current_char(), Ch(c) if c.is_numeric()) {
            value.push(self.next_char().into());
        }

        value
    }

    fn consume_chars(&mut self, mut len: usize) -> String {
        let mut value = String::new();

        while len > 0 {
            value.push(self.next_char().into());
            len -= 1;
        }

        value
    }

    fn consume_whitespace(&mut self) {
        while self.current_char().is_whitespace() {
            self.next_char();
        }
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
        if let Ch(char) = self.current_char() {
            (char >= get_unicode_char(&UnicodeChar::Null)
                && char <= get_unicode_char(&UnicodeChar::Backspace))
                || (char >= get_unicode_char(&UnicodeChar::ShiftOut)
                    && char <= get_unicode_char(&UnicodeChar::InformationSeparatorOne))
                || char == get_unicode_char(&UnicodeChar::Tab)
                || char == get_unicode_char(&UnicodeChar::Delete)
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
        let current_char = self.current_char();
        for char in chars {
            if current_char == Ch(char) {
                return true;
            }
        }

        false
    }

    fn current_char(&self) -> Character {
        self.stream.look_ahead(0)
    }

    pub fn tell(&self) -> usize {
        self.cur_location.offset() as usize
    }

    pub fn slice(&self, start: usize, end: usize) -> String {
        let mut s = String::new();
        for c in self.stream.get_slice(start, end) {
            if let Ch(c) = c {
                s.push(*c);
            }
        }

        s
    }

    fn next_char(&mut self) -> Character {
        if self.stream.eof() {
            return Character::StreamEnd;
        }

        let c = self.stream.read();
        self.cur_location.inc_offset();
        if c == Ch('\n') {
            self.cur_location.inc_line();
            self.cur_location.set_column(1);
        } else {
            self.cur_location.inc_column();
        }

        // advance position in the stream
        self.stream.next();

        c
    }

    fn look_ahead_slice(&self, len: usize) -> String {
        let mut s = String::new();

        for i in 0..len {
            match self.stream.look_ahead(i) {
                Ch(c) => s.push(c),
                _ => break,
            }
        }

        s
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use gosub_shared::byte_stream::Encoding;

    macro_rules! assert_token_eq {
        ($t1:expr, $t2:expr) => {
            assert_eq!($t1.token_type, $t2.token_type)
        };
    }

    #[test]
    fn parse_comment() {
        let mut chars = ByteStream::new();
        chars.read_from_str("/* css comment */", Some(Encoding::UTF8));
        chars.close();

        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());
        tokenizer.consume_comment();

        assert!(chars.eof())
    }

    #[test]
    fn parse_numbers() {
        let mut chars = ByteStream::new();

        let num_tokens = vec![
            // ("12", 12.0),
            // ("+34", 34.0),
            // ("-56", -56.0),
            // ("7.8", 7.8),
            // ("-9.10", -9.10),
            // ("0.0001", 0.0001),
            ("1e+1", 1e+1),
            ("1e1", 1e1),
            ("1e-1", 1e-1),
        ];

        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

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
        let mut chars = ByteStream::new();

        let ident_tokens = vec![
            ("-ident", "-ident"),
            ("ide  nt", "ide"),
            ("_123-ident", "_123-ident"),
            ("_123\\ident", "_123ident"),
        ];

        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

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
            let mut chars = ByteStream::new();

            let escaped_chars = vec![
                ("\\005F ", get_unicode_char(&UnicodeChar::LowLine)),
                ("\\2A", '*'),
                (
                    "\\000000 ",
                    get_unicode_char(&UnicodeChar::ReplacementCharacter),
                ),
                (
                    "\\FFFFFF ",
                    get_unicode_char(&UnicodeChar::ReplacementCharacter),
                ),
                (
                    "\\10FFFF ",
                    get_unicode_char(&UnicodeChar::ReplacementCharacter),
                ),
            ];

            let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

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
        let mut chars = ByteStream::new();

        let urls = vec![
            (
                "url(https://gosub.io/)",
                Token::new_url("https://gosub.io/", Location::default()),
            ),
            (
                "url(  gosub.io   )",
                Token::new_url("gosub.io", Location::default()),
            ),
            (
                "url(gosub\u{002E}io)",
                Token::new_url("gosub.io", Location::default()),
            ),
            (
                "url(gosub\u{FFFD}io)",
                Token::new_url("gosub�io", Location::default()),
            ),
            (
                "url(gosub\u{0000}io)",
                Token::new_bad_url("gosub", Location::default()),
            ),
        ];

        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for (raw_url, url_token) in urls {
            tokenizer
                .stream
                .read_from_str(raw_url, Some(Encoding::UTF8));
            assert_token_eq!(tokenizer.consume_ident_like_seq(), url_token);
        }
    }

    #[test]
    fn parse_function_tokens() {
        let mut chars = ByteStream::new();

        let functions = vec![
            ("url(\"", Token::new_function("url", Location::default())),
            ("url( \"", Token::new_function("url", Location::default())),
            ("url(\'", Token::new_function("url", Location::default())),
            ("url( \'", Token::new_function("url", Location::default())),
            ("url(\"", Token::new_function("url", Location::default())),
            ("attr('", Token::new_function("attr", Location::default())),
            (
                "rotateX(    '",
                Token::new_function("rotateX", Location::default()),
            ),
            (
                "rotateY(    \"",
                Token::new_function("rotateY", Location::default()),
            ),
            ("-rgba(", Token::new_function("-rgba", Location::default())),
            (
                "--rgba(",
                Token::new_function("--rgba", Location::default()),
            ),
            (
                "-\\26 -rgba(",
                Token::new_function("-&-rgba", Location::default()),
            ),
            ("0rgba()", Token::new_function("0rgba", Location::default())),
            (
                "-0rgba()",
                Token::new_function("-0rgba", Location::default()),
            ),
            ("_rgba()", Token::new_function("_rgba", Location::default())),
            ("rgbâ()", Token::new_function("rgbâ", Location::default())),
            (
                "\\30rgba()",
                Token::new_function("0rgba", Location::default()),
            ),
            ("rgba ()", Token::new_ident("rgba", Location::default())),
            (
                "-\\-rgba(",
                Token::new_function("--rgba", Location::default()),
            ),
        ];

        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for (raw_function, function_token) in functions {
            tokenizer
                .stream
                .read_from_str(raw_function, Some(Encoding::UTF8));
            assert_token_eq!(tokenizer.consume_ident_like_seq(), function_token);
        }
    }

    #[test]
    fn parser_numeric_token() {
        let mut chars = ByteStream::new();

        let numeric_tokens = vec![
            (
                "1.1rem",
                Token::new_dimension(1.1, "rem", Location::default()),
            ),
            ("1px", Token::new_dimension(1.0, "px", Location::default())),
            ("1em", Token::new_dimension(1.0, "em", Location::default())),
            ("1 em", Token::new_number(1.0, Location::default())),
            ("1   em", Token::new_number(1.0, Location::default())),
            ("100%", Token::new_percentage(100.0, Location::default())),
            ("42", Token::new_number(42.0, Location::default())),
            ("18 px", Token::new_number(18.0, Location::default())),
        ];

        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for (raw_token, token) in numeric_tokens {
            tokenizer
                .stream
                .read_from_str(raw_token, Some(Encoding::UTF8));
            assert_token_eq!(tokenizer.consume_numeric_token(), token);
        }
    }

    #[test]
    fn parse_string_tokens() {
        let mut stream = ByteStream::new();

        let string_tokens = vec![
            (
                "'line\nnewline'",
                Token::new_bad_string("line", Location::default()),
            ),
            (
                "\"double quotes\"",
                Token::new_quoted_string("double quotes", Location::default()),
            ),
            (
                "\'single quotes\'",
                Token::new_quoted_string("single quotes", Location::default()),
            ),
            (
                "#hash#",
                Token::new_quoted_string("hash", Location::default()),
            ),
            (
                "\"eof",
                Token::new_quoted_string("eof", Location::default()),
            ),
            ("\"\"", Token::new_quoted_string("", Location::default())),
        ];

        for (raw_string, string_token) in string_tokens {
            let mut tokenizer = Tokenizer::new(&mut stream, Location::default());
            tokenizer
                .stream
                .read_from_str(raw_string, Some(Encoding::UTF8));
            tokenizer.stream.close();

            let t = tokenizer.consume_string_token();
            assert_token_eq!(t, string_token);
        }
    }

    #[test]
    fn produce_stream_of_double_quoted_strings() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "\"\" \"Lorem 'îpsum'\" \"a\\\nb\" \"a\nb \"eof",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            // `\"\"`
            Token::new_quoted_string("", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // \"Lorem 'îpsum'\"
            Token::new_quoted_string("Lorem 'îpsum'", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `\"a\\\nb\"`
            Token::new_quoted_string("ab", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_bad_string("a", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_ident("b", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_quoted_string("eof", Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn procude_stream_of_single_quoted_strings() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "'' 'Lorem \"îpsum\"' 'a\\\nb' 'a\nb 'eof",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            // `\"\"`
            Token::new_quoted_string("", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // \"Lorem 'îpsum'\"
            Token::new_quoted_string("Lorem \"îpsum\"", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `\"a\\\nb\"`
            Token::new_quoted_string("ab", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_bad_string("a", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_ident("b", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_quoted_string("eof", Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_urls_with_strings() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "url( '') url('Lorem \"îpsum\"'\n) url('a\\\nb' ) url('a\nb) url('eof",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            // `url( '')`
            Token::new_function("url", Location::default()),
            Token::new_quoted_string("", Location::default()),
            Token::new(TokenType::RParen, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `url('Lorem \"îpsum\"'\n)`
            Token::new_function("url", Location::default()),
            Token::new_quoted_string("Lorem \"îpsum\"", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::RParen, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `url('a\\\nb' )`
            Token::new_function("url", Location::default()),
            Token::new_quoted_string("ab", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::RParen, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `url('a\nb)`
            Token::new_function("url", Location::default()),
            Token::new_bad_string("a", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_ident("b", Location::default()),
            Token::new(TokenType::RParen, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `url('eof`
            Token::new_function("url", Location::default()),
            Token::new_quoted_string("eof", Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn produce_valid_stream_of_css_tokens() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
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
        chars.close();

        let tokens = vec![
            // 1st css rule
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_id_hash("header", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_delim('.', Location::default()),
            Token::new_ident("nav", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::LCurly, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_ident("font-size", Location::default()),
            Token::new(TokenType::Colon, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_dimension(1.1, "rem", Location::default()),
            Token::new(TokenType::Semicolon, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::RCurly, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // 2nd css rule (AtRule)
            Token::new_atkeyword("media", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_ident("screen", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::LParen, Location::default()),
            Token::new_ident("max-width", Location::default()),
            Token::new(TokenType::Colon, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_dimension(200.0, "px", Location::default()),
            Token::new(TokenType::RParen, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::LCurly, Location::default()),
            Token::new(TokenType::RCurly, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // 3rd css declaration
            Token::new_ident("content", Location::default()),
            Token::new(TokenType::Colon, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_quoted_string("me & you", Location::default()),
            Token::new(TokenType::Semicolon, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // 4th css declaration
            Token::new_ident("background", Location::default()),
            Token::new(TokenType::Colon, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_url("https://gosub.io", Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        tokenizer.consume_whitespace();
        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }
    }

    #[test]
    fn parse_rgba_expr() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "
            rgba(255, 50%, 0%, 1)
        ",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_function("rgba", Location::default()),
            Token::new_number(255.0, Location::default()),
            Token::new(TokenType::Comma, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_percentage(50.0, Location::default()),
            Token::new(TokenType::Comma, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_percentage(0.0, Location::default()),
            Token::new(TokenType::Comma, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_number(1.0, Location::default()),
            Token::new(TokenType::RParen, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }
    }

    #[test]
    fn parse_cdo_and_cdc() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "/* CDO/CDC are not special */ <!-- --> {}",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::Cdo, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::Cdc, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::LCurly, Location::default()),
            Token::new(TokenType::RCurly, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }
    }

    #[test]
    fn parse_spaced_comments() {
        let mut chars = ByteStream::new();

        chars.read_from_str("/*/*///** /* **/*//* ", Some(Encoding::UTF8));
        chars.close();

        let tokens = vec![
            Token::new_delim('/', Location::default()),
            Token::new_delim('*', Location::default()),
            Token::new_delim('/', Location::default()),
            Token::new(TokenType::Eof, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            let t = tokenizer.consume_token();
            assert_token_eq!(t, token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_all_whitespaces() {
        let mut chars = ByteStream::new();

        chars.read_from_str("  \t\t\r\n\nRed ", Some(Encoding::UTF8));
        chars.close();

        let tokens = vec![
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_ident("Red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::Eof, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_at_keywords() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "@media0 @-Media @--media @0media @-0media @_media @.media @medİa @\\30 media\\",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            Token::new_atkeyword("media0", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_atkeyword("-Media", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_atkeyword("--media", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `@0media` => [@, 0, meida]
            Token::new_delim('@', Location::default()),
            Token::new_dimension(0.0, "media", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `@-0media` => [@, -0, meida]
            Token::new_delim('@', Location::default()),
            Token::new_dimension(-0.0, "media", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `@_media`
            Token::new_atkeyword("_media", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `@.meida` => [@, ., media]
            Token::new_delim('@', Location::default()),
            Token::new_delim('.', Location::default()),
            Token::new_ident("media", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `@medİa`
            Token::new_atkeyword("medİa", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `@\\30 media`
            Token::new_atkeyword("0media\u{FFFD}", Location::default()),
            Token::new(TokenType::Eof, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_id_selectors() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "#red0 #-Red #--red #-\\-red #0red #-0red #_Red #.red #rêd #êrd #\\.red\\",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            Token::new_id_hash("red0", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_id_hash("-Red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_id_hash("--red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `#--\\red`
            Token::new_id_hash("--red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `#0red` => 0red
            Token::new_hash("0red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `#-0red`
            Token::new_hash("-0red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `#_Red`
            Token::new_id_hash("_Red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `#.red` => [#, ., red]
            Token::new_delim('#', Location::default()),
            Token::new_delim('.', Location::default()),
            Token::new_ident("red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `#rêd`
            Token::new_id_hash("rêd", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `#êrd`
            Token::new_id_hash("êrd", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `#\\.red\\`
            Token::new_id_hash(".red\u{FFFD}", Location::default()),
            Token::new(TokenType::Eof, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_dimension_tokens() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "12red0 12.0-red 12--red 12-\\-red 120red 12-0red 12\\0000red 12_Red 12.red 12rêd",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            // `12red0`
            Token::new_dimension(12.0, "red0", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `12.0-red`
            Token::new_dimension(12.0, "-red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `12--red`
            Token::new_dimension(12.0, "--red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `12-\\-red`
            Token::new_dimension(12.0, "--red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `120red`
            Token::new_dimension(120.0, "red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `12-0red` => [12, -0red]
            Token::new_number(12.0, Location::default()),
            Token::new_dimension(-0.0, "red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `12\u{0000}red`
            Token::new_dimension(12.0, "\u{FFFD}red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `12_Red`
            Token::new_dimension(12.0, "_Red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `12.red` => [12, ., red]
            Token::new_number(12.0, Location::default()),
            Token::new_delim('.', Location::default()),
            Token::new_ident("red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `12rêd`
            Token::new_dimension(12.0, "rêd", Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_dimension_tokens_2() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "12e2px +34e+1px -45E-0px .68e+3px +.79e-1px -.01E2px 2.3E+1px +45.0e6px -0.67e0px",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            // `12e2px`
            Token::new_dimension(1200.0, "px", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `+34e+1px`
            Token::new_dimension(340.0, "px", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `-45E-0px`
            Token::new_dimension(-45.0, "px", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `.68e+3px`
            Token::new_dimension(680.0, "px", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `+.79e-1px`
            Token::new_dimension(0.079, "px", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `-.01E2px`
            Token::new_dimension(-1.0, "px", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `2.3E+1px`
            Token::new_dimension(23.0, "px", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `+45.0e6px`
            Token::new_dimension(45000000.0, "px", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `-0.67e0px`
            Token::new_dimension(-0.67, "px", Location::default()),
            Token::new(TokenType::Eof, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_percentage() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "12e2% +34e+1% -45E-0% .68e+3% +.79e-1% -.01E2% 2.3E+1% +45.0e6% -0.67e0%",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            // `12e2%`
            Token::new_percentage(1200.0, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `+34e+1%`
            Token::new_percentage(340.0, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `-45E-0%`
            Token::new_percentage(-45.0, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `.68e+3%`
            Token::new_percentage(680.0, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `+.79e-1%`
            Token::new_percentage(0.079, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `-.01E2%`
            Token::new_percentage(-1.0, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `2.3E+1%`
            Token::new_percentage(23.0, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `+45.0e6%`
            Token::new_percentage(45000000.0, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `-0.67e0%`
            Token::new_percentage(-0.67, Location::default()),
            Token::new(TokenType::Eof, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_css_seq_1() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "a:not([href^=http\\:],  [href ^=\t'https\\:'\n]) { color: rgba(0%, 100%, 50%); }",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            Token::new_ident("a", Location::default()),
            Token::new(TokenType::Colon, Location::default()),
            Token::new_function("not", Location::default()),
            Token::new(TokenType::LBracket, Location::default()),
            Token::new_ident("href", Location::default()),
            Token::new_delim('^', Location::default()),
            Token::new_delim('=', Location::default()),
            Token::new_ident("http:", Location::default()),
            Token::new(TokenType::RBracket, Location::default()),
            Token::new(TokenType::Comma, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::LBracket, Location::default()),
            Token::new_ident("href", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_delim('^', Location::default()),
            Token::new_delim('=', Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_quoted_string("https:", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::RBracket, Location::default()),
            Token::new(TokenType::RParen, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::LCurly, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_ident("color", Location::default()),
            Token::new(TokenType::Colon, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_function("rgba", Location::default()),
            Token::new_percentage(0.0, Location::default()),
            Token::new(TokenType::Comma, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_percentage(100.0, Location::default()),
            Token::new(TokenType::Comma, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_percentage(50.0, Location::default()),
            Token::new(TokenType::RParen, Location::default()),
            Token::new(TokenType::Semicolon, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::RCurly, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_css_seq_2() {
        let mut chars = ByteStream::new();

        chars.read_from_str("red-->/* Not CDC */", Some(Encoding::UTF8));
        chars.close();

        let tokens = vec![
            Token::new_ident("red--", Location::default()),
            Token::new_delim('>', Location::default()),
            // @todo: we need to figure this case out:
            // the next token after the '>' is a comment, but these are skipped. So they are not
            // returned on a call to `consume_token()`. Since this is the last token, it will return
            // and EOF token instead. However, when calling tokenizer.stream.eof(), it will return
            // false, since we still have to read the comment.
            // There is thus a discrepancy between the Eof token returned by `consume_token()` and
            // the actual eof from the stream. If we read the comment like we do below, we actually
            // get the correct result.
            Token::new(TokenType::Eof, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_css_seq_3() {
        let mut chars = ByteStream::new();

        chars.read_from_str("\\- red0 -red --red -\\-red\\ blue 0red -0red \\0000red _Red .red rêd r\\êd \\007F\\0080\\0081", Some(Encoding::UTF8));
        chars.close();

        let tokens = vec![
            // `\\-`
            Token::new_ident("-", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `red0`
            Token::new_ident("red0", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `-red`
            Token::new_ident("-red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `--red`
            Token::new_ident("--red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `-\\-red\\ blue`
            Token::new_ident("--red blue", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `0red`
            Token::new_dimension(0.0, "red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `-0red`
            Token::new_dimension(-0.0, "red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `\\0000red`
            Token::new_ident("\u{FFFD}red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `_Red`
            Token::new_ident("_Red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `.red` => [., red]
            Token::new_delim('.', Location::default()),
            Token::new_ident("red", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `rêd`
            Token::new_ident("rêd", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `r\\êd`
            Token::new_ident("rêd", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            // `\\007F\\0080\\0081`
            Token::new_ident("\u{7f}\u{80}\u{81}", Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn parse_css_seq_4() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "p[example=\"\\\nfoo(int x) {\\\n   this.x = x;\\\n}\\\n\"]",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            Token::new_ident("p", Location::default()),
            Token::new(TokenType::LBracket, Location::default()),
            Token::new_ident("example", Location::default()),
            Token::new_delim('=', Location::default()),
            Token::new_quoted_string("foo(int x) {   this.x = x;}", Location::default()),
            Token::new(TokenType::RBracket, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn consume_tokenizer_as_stream_of_tokens() {
        let mut chars = ByteStream::new();
        chars.read_from_str("[][]", Some(Encoding::UTF8));
        chars.close();

        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());
        tokenizer.consume_all();

        assert_token_eq!(
            tokenizer.lookahead(0),
            Token::new(TokenType::LBracket, Location::default())
        );
        assert_token_eq!(
            tokenizer.lookahead(1),
            Token::new(TokenType::RBracket, Location::default())
        );
        assert_token_eq!(
            tokenizer.lookahead(4),
            Token::new(TokenType::Eof, Location::default())
        );

        assert_token_eq!(
            tokenizer.consume(),
            Token::new(TokenType::LBracket, Location::default())
        );
        assert_token_eq!(
            tokenizer.lookahead(0),
            Token::new(TokenType::RBracket, Location::default())
        );
    }

    #[test]
    fn parse_css_seq_5() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "test { color: #123; background-color: #11223344 }",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            Token::new_ident("test", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::LCurly, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_ident("color", Location::default()),
            Token::new(TokenType::Colon, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_hash("123", Location::default()),
            Token::new(TokenType::Semicolon, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_ident("background-color", Location::default()),
            Token::new(TokenType::Colon, Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new_hash("11223344", Location::default()),
            Token::new(TokenType::Whitespace, Location::default()),
            Token::new(TokenType::RCurly, Location::default()),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            assert_token_eq!(tokenizer.consume_token(), token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn location() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "test { color: #123; background-color: #11223344 }",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            Token::new_ident("test", Location::new(1, 1, 0)),
            Token::new(TokenType::Whitespace, Location::new(1, 5, 4)),
            Token::new(TokenType::LCurly, Location::new(1, 6, 5)),
            Token::new(TokenType::Whitespace, Location::new(1, 7, 6)),
            Token::new_ident("color", Location::new(1, 8, 7)),
            Token::new(TokenType::Colon, Location::new(1, 13, 12)),
            Token::new(TokenType::Whitespace, Location::new(1, 14, 13)),
            Token::new_hash("123", Location::new(1, 15, 14)),
            Token::new(TokenType::Semicolon, Location::new(1, 19, 18)),
            Token::new(TokenType::Whitespace, Location::new(1, 20, 19)),
            Token::new_ident("background-color", Location::new(1, 21, 20)),
            Token::new(TokenType::Colon, Location::new(1, 37, 36)),
            Token::new(TokenType::Whitespace, Location::new(1, 38, 37)),
            Token::new_hash("11223344", Location::new(1, 39, 38)),
            Token::new(TokenType::Whitespace, Location::new(1, 48, 47)),
            Token::new(TokenType::RCurly, Location::new(1, 49, 48)),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            let t = tokenizer.consume_token();
            println!("{:?}", t);
            assert_eq!(t, token);
        }

        assert!(tokenizer.stream.eof());
    }

    #[test]
    fn location_multiline() {
        let mut chars = ByteStream::new();

        chars.read_from_str(
            "test {\n    color: #123;\n    background-color: #11223344\n}",
            Some(Encoding::UTF8),
        );
        chars.close();

        let tokens = vec![
            Token::new_ident("test", Location::new(1, 1, 0)),
            Token::new(TokenType::Whitespace, Location::new(1, 5, 4)),
            Token::new(TokenType::LCurly, Location::new(1, 6, 5)),
            Token::new(TokenType::Whitespace, Location::new(1, 7, 6)),
            Token::new_ident("color", Location::new(2, 5, 11)),
            Token::new(TokenType::Colon, Location::new(2, 10, 16)),
            Token::new(TokenType::Whitespace, Location::new(2, 11, 17)),
            Token::new_hash("123", Location::new(2, 12, 18)),
            Token::new(TokenType::Semicolon, Location::new(2, 16, 22)),
            Token::new(TokenType::Whitespace, Location::new(2, 17, 23)),
            Token::new_ident("background-color", Location::new(3, 5, 28)),
            Token::new(TokenType::Colon, Location::new(3, 21, 44)),
            Token::new(TokenType::Whitespace, Location::new(3, 22, 45)),
            Token::new_hash("11223344", Location::new(3, 23, 46)),
            Token::new(TokenType::Whitespace, Location::new(3, 32, 55)),
            Token::new(TokenType::RCurly, Location::new(4, 1, 56)),
        ];
        let mut tokenizer = Tokenizer::new(&mut chars, Location::default());

        for token in tokens {
            let t = tokenizer.consume_token();
            assert_eq!(t, token);
        }

        assert!(tokenizer.stream.eof());
    }
}
