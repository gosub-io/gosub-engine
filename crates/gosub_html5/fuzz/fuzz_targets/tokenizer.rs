#![no_main]

use gosub_html5::parser::errors::ErrorLogger;
use gosub_html5::tokenizer::{ParserData, Tokenizer};
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use libfuzzer_sys::fuzz_target;
use std::cell::RefCell;
use std::rc::Rc;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
        let mut stream = ByteStream::from_str(s, Encoding::UTF8);
        let mut tokenizer = Tokenizer::new(&mut stream, None, error_logger, Location::default());

        loop {
            match tokenizer.next_token(ParserData::default()) {
                Ok(tok) if tok.is_eof() => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }
});
