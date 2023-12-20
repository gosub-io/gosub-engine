// note: file should be in a shared lib

use lazy_static::lazy_static;
use std::collections::HashMap;

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

lazy_static! {
    static ref UNICODE_CHARS: HashMap<UnicodeChar, char> = HashMap::from([
        (UnicodeChar::Null, '\u{0000}'),
        (UnicodeChar::Backspace, '\u{0008}'),
        (UnicodeChar::Tab, '\u{000B}'),
        (UnicodeChar::ShiftOut, '\u{000E}'),
        (UnicodeChar::Delete, '\u{007F}'),
        (UnicodeChar::InformationSeparatorOne, '\u{001F}'),
        (UnicodeChar::LowLine, '\u{005F}'),
        (UnicodeChar::MaxAllowed, '\u{10FFFF}'),
        (UnicodeChar::ReplacementCharacter, '\u{FFFD}')
    ]);
}

pub fn get_unicode_char(char: &UnicodeChar) -> char {
    *UNICODE_CHARS.get(char).expect("Unknown unicode char.")
}
