use anyhow::Result;
use std::fs;
use std::process::exit;
use gosub_engine::css3;
use gosub_engine::css3::Css3;
use gosub_engine::css3::parser_config::ParserConfig;
use simple_logger::SimpleLogger;

fn bail(message: &str) -> ! {
    println!("{message}");
    exit(1);
}

fn main() -> Result<()> {
    let matches = clap::Command::new("Gosub CSS3 parser")
        .version("0.1.0")
        .arg(clap::Arg::new("url")
            .help("The url or file to parse")
            .required(true)
            .index(1)
        )
        .arg(clap::Arg::new("debug")
            .help("Enable debug logging")
            .short('d')
            .long("debug")
            .action(clap::ArgAction::SetTrue)
        )
        .get_matches();

    let debug = matches.get_flag("debug");
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

    let config = ParserConfig {
        source: Some("stylesheet.css".into()),
        ..Default::default()
    };

    if debug {
        SimpleLogger::new().init().unwrap();
    }

    let res = Css3::parse(css.as_str(), config);
    if res.is_err() {
        println!("{:?}", res.err().unwrap());
        return Ok(());
    }

    css3::walker::Walker.walk(&res.unwrap());

    Ok(())
}
