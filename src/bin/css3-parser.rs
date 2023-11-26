use anyhow::{anyhow, Result};
use gosub_engine::byte_stream::{ByteStream, Encoding, Stream};
use gosub_engine::css3;
use gosub_engine::css3::location::Location;
use gosub_engine::css3::parser_config::ParserConfig;
use gosub_engine::css3::{Css3, Error};
use simple_logger::SimpleLogger;
use std::fs;
use std::process::exit;

fn bail(message: &str) -> ! {
    println!("{message}");
    exit(1);
}

fn main() -> Result<()> {
    let matches = clap::Command::new("Gosub CSS3 parser")
        .version("0.1.0")
        .arg(
            clap::Arg::new("url")
                .help("The url or file to parse")
                .required(true)
                .index(1),
        )
        .arg(
            clap::Arg::new("debug")
                .help("Enable debug logging")
                .short('d')
                .long("debug")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            clap::Arg::new("tokens")
                .help("Just print the tokens")
                .long("tokens")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            clap::Arg::new("ignore-errors")
                .help("Ignore errors")
                .long("ignore-errors")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            clap::Arg::new("quiet")
                .help("Don't display AST")
                .long("quiet")
                .short('q')
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

    let debug = matches.get_flag("debug");
    let quiet = matches.get_flag("quiet");
    let ignore_errors = matches.get_flag("ignore-errors");
    let tokens = matches.get_flag("tokens");
    let url: String = matches.get_one::<String>("url").expect("url").to_string();

    let css = if url.starts_with("http://") || url.starts_with("https://") {
        // Fetch the html from the url
        let response = ureq::get(&url).call()?;
        if response.status() != 200 {
            bail(&format!(
                "Could not get url. Status code {}",
                response.status()
            ));
        }
        response.into_string()?
    } else {
        // Get html from the file
        fs::read_to_string(&url)?
    };

    if tokens {
        print_tokens(css);
        return Ok(());
    }

    let config = ParserConfig {
        source: Some("stylesheet.css".into()),
        ignore_errors,
        ..Default::default()
    };

    if debug {
        SimpleLogger::new().init().unwrap();
    }

    let result = Css3::parse(css.as_str(), config);
    if result.is_err() {
        let err = result.err().unwrap();
        let message = err.message.clone();
        display_snippet(&css, err);
        return Err(anyhow!(message));
    }

    if !quiet {
        let binding = result.unwrap();
        let walker = css3::walker::Walker::new(&binding);
        walker.walk_stdout();
    }

    Ok(())
}

fn display_snippet(css: &str, err: Error) {
    let loc = err.location.clone();
    let lines: Vec<&str> = css.split('\n').collect();
    let line_nr = loc.line() - 1;
    let col_nr = if loc.column() < 2 {
        0
    } else {
        loc.column() - 2
    };

    if col_nr > 1000 {
        println!("Error is too far to the right to display.");
        return;
    }

    // Print the previous 5 lines
    println!();
    println!();
    for n in (line_nr as i32 - 5)..(line_nr as i32) {
        if n < 0 {
            continue;
        }
        println!("{:<5}|{}", n + 1, lines[n as usize]);
    }

    // Print the line with the error and a pointer to the error
    println!("{:<5}|{}", line_nr + 1, lines[line_nr as usize]);
    println!("   ---{}^", "-".repeat(col_nr as usize));

    // Print the next 5 lines
    for n in line_nr + 1..line_nr + 6 {
        if n > lines.len() as u32 - 1 {
            continue;
        }
        println!("{:<5}|{}", n + 1, lines[n as usize]);
    }
    println!();
    println!();
}

fn print_tokens(css: String) {
    let mut it = ByteStream::new();
    it.read_from_str(&css, Some(Encoding::UTF8));
    it.close();

    let mut tokenizer = css3::tokenizer::Tokenizer::new(&mut it, Location::default());
    loop {
        let token = tokenizer.consume();
        println!("{:?}", token);

        if token.token_type == css3::tokenizer::TokenType::Eof {
            break;
        }
    }
}
