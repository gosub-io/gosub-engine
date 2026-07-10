// @todo: this belongs in a shared library.
#[derive(Debug, Eq, Hash, PartialEq)]
pub enum UnicodeChar {
    Null,
    Backspace,
    Tab,
    ShiftOut,
    Delete,
    InformationSeparatorOne,
    #[allow(dead_code)] // only constructed by the escape-sequence tests
    LowLine,
    MaxAllowed,
    ReplacementCharacter,
}

pub fn get_unicode_char(char: &UnicodeChar) -> char {
    match char {
        UnicodeChar::Null => '\u{0000}',
        UnicodeChar::Backspace => '\u{0008}',
        UnicodeChar::Tab => '\u{000B}',
        UnicodeChar::ShiftOut => '\u{000E}',
        UnicodeChar::Delete => '\u{007F}',
        UnicodeChar::InformationSeparatorOne => '\u{001F}',
        UnicodeChar::LowLine => '\u{005F}',
        UnicodeChar::MaxAllowed => '\u{10FFFF}',
        UnicodeChar::ReplacementCharacter => '\u{FFFD}',
    }
}
