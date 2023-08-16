use crate::html5_parser::token::Token;
use std::fmt::{Display, Formatter};

pub(crate) trait Emitter: Display {
    fn emit(&mut self, t: Token);
}

// Emitter that will send the output to a string
struct StrEmitter {
    output: String,
}

impl StrEmitter {
    pub fn new() -> Self {
        StrEmitter {
            output: String::new(),
        }
    }
}

impl Display for StrEmitter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.output)
    }
}

impl Emitter for StrEmitter {
    fn emit(&mut self, _t: Token) {
        // self.output.add(&*t.to_string());
    }
}

// Default emitter that will emit tokens to the std output
pub struct IoEmitter {}

impl IoEmitter {
    pub fn new() -> Self {
        IoEmitter {}
    }
}

impl Display for IoEmitter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "")
    }
}

// Implement the emit() function
impl Emitter for IoEmitter {
    fn emit(&mut self, t: Token) {
        println!("{}", t.to_string());
    }
}

#[cfg(test)]
mod test {

    // #[test]
    // fn test_emit() {
    //     let e = StrEmitter::new();
    //     e.emit(Token::String(String::from("hello world")));
    //     assert_eq!(e.output, "hello world");
    //
    //     let e = StrEmitter::new();
    //     e.emit(Token::StartTag(StartTag::new("tag", true, None, "")));
    //     assert_eq!(e.output, "<tag/>");
    // }
}
