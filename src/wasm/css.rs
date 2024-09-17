use wasm_bindgen::prelude::wasm_bindgen;

use gosub_css3::parser_config::ParserConfig;
use gosub_css3::tokenizer::{TokenType, Tokenizer};
use gosub_css3::walker::Walker;
use gosub_css3::{Css3, Error};
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};

#[wasm_bindgen]
pub struct CssOptions {
    tokens: bool,
    ignore_errors: bool,
}

#[wasm_bindgen]
impl CssOptions {
    #[wasm_bindgen(constructor)]
    pub fn new(tokens: bool, ignore_errors: bool) -> Self {
        Self { tokens, ignore_errors }
    }
}

#[wasm_bindgen]
impl CssOutput {
    pub fn to_string(&self) -> String {
        format!("{}\n{}", self.out, self.tokens)
    }

    pub fn out(&self) -> String {
        self.out.clone()
    }

    pub fn tokens(&self) -> String {
        self.tokens.clone()
    }
}

#[wasm_bindgen]
pub struct CssOutput {
    tokens: String,
    out: String,
}

#[wasm_bindgen]
pub fn css3_parser(input: &str, opts: CssOptions) -> CssOutput {
    let tokens = if opts.tokens {
        print_tokens(&input)
    } else {
        String::new()
    };

    let config = ParserConfig {
        source: Some("stylesheet.css".into()),
        ignore_errors: opts.ignore_errors,
        ..Default::default()
    };

    match Css3::parse(input, config) {
        Ok(parsed) => {
            let out = Walker::new(&parsed).walk_to_string();
            CssOutput { tokens, out }
        }
        Err(err) => {
            let snippet = display_snippet(&input, err);
            CssOutput { tokens, out: snippet }
        }
    }
}

fn display_snippet(css: &str, err: Error) -> String {
    let loc = err.location.clone();
    let lines: Vec<&str> = css.split('\n').collect();
    let line_nr = loc.line - 1;
    let col_nr = if loc.column < 2 { 0 } else { loc.column - 2 };

    if col_nr > 1000 {
        return String::from("Error is too far to the right to display.");
    }
    let mut out = String::new();

    // Print the previous 5 lines
    out.push('\n');
    out.push('\n');
    for n in (line_nr as i32 - 5)..(line_nr as i32) {
        if n < 0 {
            continue;
        }
        out.push_str(&format!("{:<5}|{}\n", n + 1, lines[n as usize]));
    }

    // Print the line with the error and a pointer to the error
    out.push_str(&format!("{:<5}|{}\n", line_nr + 1, lines[line_nr as usize]));
    out.push_str(&format!("   ---{}^\n", "-".repeat(col_nr as usize)));

    // Print the next 5 lines
    for n in line_nr + 1..line_nr + 6 {
        if n > lines.len() - 1 {
            continue;
        }
        out.push_str(&format!("{:<5}|{}\n", n + 1, lines[n as usize]));
    }

    out.push('\n');
    out.push('\n');

    out
}

fn print_tokens(css: &str) -> String {
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(css, Some(Encoding::UTF8));
    stream.close();

    let mut tokenizer = Tokenizer::new(&mut stream, Location::default());

    let mut out = String::new();

    loop {
        let token = tokenizer.consume();
        out.push_str(&format!("{:?}\n", token));

        if token.token_type == TokenType::Eof {
            break;
        }
    }

    out
}
