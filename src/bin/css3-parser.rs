use anyhow::{anyhow, bail, Result};
use gosub_css3::parser_config::ParserConfig;
use gosub_css3::tokenizer::{TokenType, Tokenizer};
use gosub_css3::{walker, Css3, Error};
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use simple_logger::SimpleLogger;
use std::fs;
use std::time::Instant;

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
            bail!(format!(
                "Could not get url. Status code {}",
                response.status()
            ));
        }
        response.into_string()?
    } else if url.starts_with("file://") {
        let path = url.trim_start_matches("file://");
        fs::read_to_string(path)?
    } else {
        bail!("Unsupported url scheme: {}", url);
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

    let now = Instant::now();
    let result = Css3::parse(css.as_str(), config);
    let elapsed_time = now.elapsed();
    println!(
        "Running css3 parser of ({}) took {} ms.",
        byte_size(css.len() as u64),
        elapsed_time.as_millis()
    );

    if result.is_err() {
        let err = result.err().unwrap();
        let message = err.message.clone();
        display_snippet(&css, err);

        return Err(anyhow!(message));
    }

    if !quiet {
        let binding = result.unwrap();
        let walker = walker::Walker::new(&binding);
        walker.walk_stdout();
    }

    Ok(())
}

fn display_snippet(css: &str, err: Error) {
    let loc = err.location.clone();
    let lines: Vec<&str> = css.split('\n').collect();
    let line_nr = loc.line - 1;
    let col_nr = if loc.column < 2 { 0 } else { loc.column - 2 };

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
    println!("{:<5}|{}", line_nr + 1, lines[line_nr]);
    println!("   ---{}^", "-".repeat(col_nr));

    // Print the next 5 lines
    for n in line_nr + 1..line_nr + 6 {
        if n > lines.len() - 1 {
            continue;
        }
        println!("{:<5}|{}", n + 1, lines[n]);
    }
    println!();
    println!();
}

fn print_tokens(css: String) {
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(&css, Some(Encoding::UTF8));
    stream.close();

    let mut tokenizer = Tokenizer::new(&mut stream, Location::default());
    loop {
        let token = tokenizer.consume();
        println!("{:?}", token);

        if token.token_type == TokenType::Eof {
            break;
        }
    }
}

/// Returns a human-readable byte size
fn byte_size(bytes: u64) -> String {
    let sizes = ["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let i = (bytes as f64).log2().floor() as i32 / 10;
    format!(
        "{:.2} {}",
        bytes as f64 / 2_f64.powi(i * 10),
        sizes[i as usize]
    )
}
