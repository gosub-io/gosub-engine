use anyhow::{anyhow, bail, Result};
use gosub_css3::tokenizer::{TokenType, Tokenizer};
use gosub_css3::Css3;
use gosub_interface::css3::CssOrigin;
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use gosub_shared::config::ParserConfig;
use gosub_shared::errors::CssError;
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
            clap::Arg::new("ignore-errors")
                .help("Ignore errors")
                .long("ignore-errors")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            clap::Arg::new("match-values")
                .help("Check if values given for each property matches its property syntax")
                .long("match-values")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            clap::Arg::new("tokenizer")
                .help("Just print the tokens generated by the tokenizer")
                .long("tokenizer")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

    let debug = matches.get_flag("debug");
    let ignore_errors = matches.get_flag("ignore-errors");
    let match_values = matches.get_flag("match-values");
    let url: String = matches.get_one::<String>("url").expect("url").to_string();
    let display_tokenizer = matches.get_flag("tokenizer");

    let css = if url.starts_with("http://") || url.starts_with("https://") {
        // Fetch the html from the url
        let mut response = ureq::get(&url).call()?;
        if response.status() != 200 {
            bail!(format!("Could not get url. Status code {}", response.status()));
        }
        response.body_mut().read_to_string()?
    } else if url.starts_with("file://") {
        let path = url.trim_start_matches("file://");
        fs::read_to_string(path)?
    } else {
        bail!("Unsupported url scheme: {}", url);
    };

    if debug {
        SimpleLogger::new().init().unwrap();
    }

    let config = ParserConfig {
        source: Some(url.clone()),
        ignore_errors,
        match_values,
        ..Default::default()
    };

    if display_tokenizer {
        print_tokens(&css);
        return Ok(());
    }

    let now = Instant::now();
    let result = Css3::parse_str(&css, config, CssOrigin::User, url.as_str());
    let elapsed_time = now.elapsed();
    println!(
        "Running css3 parser of ({}) took {} ms.",
        byte_size(css.len() as u64),
        elapsed_time.as_millis()
    );

    if result.is_err() {
        // Err is a anyhow::Error, which wraps a Css3::Error
        let err = result.err().unwrap();
        let message = err.to_string();
        display_snippet(&css, err);

        return Err(anyhow!(message));
    }

    let tree = result.unwrap();

    println!("\n------- Log messages -------");
    for log in tree.parse_log.iter() {
        println!("{}", log);
    }

    Ok(())
}

/// Print snippet where the error occurred
fn display_snippet(css: &str, err: CssError) {
    let loc = match err.location {
        Some(l) => l,
        None => {
            println!("Error: {}", err.message);
            return;
        }
    };

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

/// Print tokens generated by the tokenizer
fn print_tokens(css: &str) {
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(css, Some(Encoding::UTF8));
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

/// Returns a human-readable byte size (1024-based)
fn byte_size(bytes: u64) -> String {
    let sizes = ["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let i = (bytes as f64).log(1024.0).floor() as usize;
    format!("{:.2} {}", bytes as f64 / 1024_f64.powi(i as i32), sizes[i])
}
