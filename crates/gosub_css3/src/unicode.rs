// note: file should be in a shared lib

#[allow(clippy::module_name_repetitions)]
pub struct UnicodeChar;

impl UnicodeChar {
    pub const NULL: char = '\u{0000}';
    pub const BACKSPACE: char = '\u{0008}';
    pub const TAB: char = '\u{000B}';
    pub const SHIFT_OUT: char = '\u{000E}';
    pub const DELETE: char = '\u{007F}';
    pub const INFORMATION_SEPARATOR_ONE: char = '\u{001F}';
    #[allow(dead_code)] //TODO: why is this here?
    pub const LOW_LINE: char = '\u{005F}';
    pub const MAX_ALLOWED: char = '\u{10FFFF}';
    pub const REPLACEMENT_CHARACTER: char = '\u{FFFD}';
}
