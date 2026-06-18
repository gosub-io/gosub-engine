use anyhow::{anyhow, bail, Result};
use gosub_css3::Css3;
use gosub_interface::css3::CssOrigin;
use gosub_shared::config::ParserConfig;
use log::LevelFilter;
use simple_logger::SimpleLogger;
use std::path::Path;

/// Parse a single CSS source (local file or http(s):// URL) and report the result.
///
/// The parser is run with `ignore_errors` enabled so it does not stop at the first
/// problem; instead every rule it cannot parse is emitted as a `WARN` via the logger
/// (the same `gosub_css3::parser::rule` warnings you already see in the renderer examples).
fn main() -> Result<()> {
    // Default to WARN so the only output is the parser's warnings/errors. Override with
    // RUST_LOG (e.g. RUST_LOG=trace) for deeper inspection.
    SimpleLogger::new().with_level(LevelFilter::Warn).env().init()?;

    let source = match std::env::args().nth(1) {
        Some(s) => s,
        None => bail!("usage: css-check <file|http(s)-url>"),
    };

    // Accept either a real URL or a local file path.
    let url = match url::Url::parse(&source) {
        Ok(u) if matches!(u.scheme(), "http" | "https" | "file") => u,
        _ => url::Url::from_file_path(Path::new(&source).canonicalize().unwrap_or_else(|_| source.clone().into()))
            .map_err(|_| anyhow!("not a valid URL or file path: {source}"))?,
    };

    let response = gosub_net::net::simple::sync_fetch(&url)?;
    if !response.is_ok() {
        bail!("could not fetch {source} (status {})", response.status);
    }
    let css = String::from_utf8_lossy(&response.body).into_owned();

    let config = ParserConfig {
        source: Some(source.clone()),
        ignore_errors: true,
        ..Default::default()
    };

    match Css3::parse_str(&css, config, CssOrigin::Author, &source) {
        Ok(sheet) => {
            println!("Parsed {source}: {} rule(s).", sheet.rules.len());
            Ok(())
        }
        Err(e) => bail!("failed to parse {source}: {}", e.message),
    }
}
