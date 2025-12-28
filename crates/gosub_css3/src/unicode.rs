// note: file should be in a shared lib

use std::collections::HashMap;
use std::sync::LazyLock;

// note: should be shared
#[derive(Debug, Eq, Hash, PartialEq)]
pub enum UnicodeChar {
    Null,
    Backspace,
    Tab,
    ShiftOut,
    Delete,
    InformationSeparatorOne,
    LowLine,
    MaxAllowed,
    ReplacementCharacter,
}

static UNICODE_CHARS: LazyLock<HashMap<UnicodeChar, char>> = LazyLock::new(|| {
    HashMap::from([
        (UnicodeChar::Null, '\u{0000}'),
        (UnicodeChar::Backspace, '\u{0008}'),
        (UnicodeChar::Tab, '\u{000B}'),
        (UnicodeChar::ShiftOut, '\u{000E}'),
        (UnicodeChar::Delete, '\u{007F}'),
        (UnicodeChar::InformationSeparatorOne, '\u{001F}'),
        (UnicodeChar::LowLine, '\u{005F}'),
        (UnicodeChar::MaxAllowed, '\u{10FFFF}'),
        (UnicodeChar::ReplacementCharacter, '\u{FFFD}'),
    ])
});

pub fn get_unicode_char(char: &UnicodeChar) -> char {
    UNICODE_CHARS.get(char).copied().unwrap_or('\u{FFFD}')
}
