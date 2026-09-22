//! Where parsing a large stylesheet spends memory: what it keeps, and what it only borrows.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use gosub_css3::Css3;
use gosub_interface::css3::CssOrigin;
use gosub_shared::config::ParserConfig;

fn resident() -> usize {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    statm
        .split_whitespace()
        .nth(1)
        .and_then(|pages| pages.parse::<usize>().ok())
        .unwrap_or(0)
        * 4096
}

fn mb(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/data/css3-data/data.css");
    let css = std::fs::read_to_string(path).expect("the 2.2 MB sheet should exist");
    println!(
        "AST node sizes: Node {} = NodeType {} + Location {} bytes",
        size_of::<gosub_css3::node::Node>(),
        size_of::<gosub_css3::node::NodeType>(),
        size_of::<gosub_shared::byte_stream::Location>(),
    );
    let start = resident();
    println!("resident with the 2.2 MB source in hand: {}", mb(start));

    // Several parses in one process, so the comparison is not dominated by start-up.
    let mut elapsed = std::time::Duration::ZERO;
    let mut sheet = None;
    for _ in 0..5 {
        let started = std::time::Instant::now();
        let config = ParserConfig {
            ignore_errors: true,
            ..Default::default()
        };
        let parsed = Css3::parse_str(&css, config, CssOrigin::Author, "data.css").expect("parse");
        elapsed += started.elapsed();
        sheet = Some(parsed);
    }
    let sheet = sheet.expect("parsed");
    println!(
        "parse time: {:.1} ms per parse (mean of 5)",
        elapsed.as_secs_f64() * 1000.0 / 5.0
    );
    let after = resident();
    println!(
        "after parsing ({} rules): {} (+{})",
        sheet.rules.len(),
        mb(after),
        mb(after - start)
    );

    // What survives is the stylesheet; everything the parser built on the way - the token
    // stream and the AST - is gone by now. If the difference above is far larger than the
    // stylesheet itself, the gap is memory the allocator is holding rather than data we keep.
    drop(sheet);
    println!("after dropping the stylesheet: {}", mb(resident()));
}
