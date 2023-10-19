// note: input_stream should come from a shared lib.
use crate::html5_parser::input_stream::InputStream;

#[derive(Debug, PartialEq)]
pub enum NumberKind {
    Number,
    Integer,
}

#[derive(Debug, PartialEq)]
pub struct Number {
    kind: NumberKind,
    value: f32,
}

impl Number {
    pub fn new(kind: NumberKind, value: f32) -> Number {
        Number { kind, value }
    }
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

    /// 4.3.12. [Consume a number](https://www.w3.org/TR/css-syntax-3/#consume-number)
    pub fn consume_number(&mut self) -> Number {
        let mut value = String::new();
        let mut kind = NumberKind::Integer;
        let lookahead = self.stream.current_char().utf8();

        if lookahead == '+' || lookahead == '-' {
            value.push(self.stream.read_char().utf8());
        }

        value.push_str(&self.consume_digits());

        if self.stream.current_char().utf8() == '.' && self.stream.next_char().utf8().is_numeric() {
            value.push_str(&self.consume_chars(2));
            kind = NumberKind::Number;
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
            kind = NumberKind::Number;

            if self.stream.current_char().utf8() == '-' || self.stream.current_char().utf8() == '+'
            {
                value.push(self.stream.read_char().utf8());
            }
        }

        value.push_str(&self.consume_digits());

        Number::new(kind, value.parse().expect("failed to parse number"))
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
}

#[cfg(test)]
mod test {
    use crate::html5_parser::input_stream::Encoding;

    use super::*;

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
        is.read_from_str(
            "12#+34#-56#7.8#-9.10#0.0001#1e+1#1e1#1e-1",
            Some(Encoding::UTF8),
        );

        let mut tokenizer = Tokenizer::new(&mut is);

        let expected_numbers = vec![
            Number::new(NumberKind::Integer, 12.0),
            Number::new(NumberKind::Integer, 34.0),
            Number::new(NumberKind::Integer, -56.0),
            Number::new(NumberKind::Number, 7.8),
            Number::new(NumberKind::Number, -9.10),
            Number::new(NumberKind::Number, 0.0001),
            Number::new(NumberKind::Number, 1e+1),
            Number::new(NumberKind::Number, 1e1),
            Number::new(NumberKind::Number, 1e-1),
        ];

        for expected_number in expected_numbers {
            assert_eq!(tokenizer.consume_number(), expected_number);
            tokenizer.stream.read_char(); // '#'
        }

        assert!(is.eof())
    }
}
