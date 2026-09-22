//! Where parsing a large stylesheet spends memory: what it keeps, and what it only borrows.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use gosub_css3::Css3;
use gosub_interface::css3::CssOrigin;
use gosub_shared::config::ParserConfig;

/// Resident memory of this process, in bytes. `VmRSS` in `/proc/self/status` is reported in
/// kB, so this needs no page size - `/proc/self/statm` counts pages, and the page size is not
/// 4 KiB everywhere (a 16 KiB aarch64 kernel would make this read four times too small).
fn resident() -> usize {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|kb| kb.parse::<usize>().ok())
        .unwrap_or(0)
        * 1024
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
        mb(after.saturating_sub(start))
    );

    // What survives is the stylesheet; everything the parser built on the way - the token
    // stream and the AST - is gone by now. If the difference above is far larger than the
    // stylesheet itself, the gap is memory the allocator is holding rather than data we keep.
    // How much of the sheet is the same value written again? Keyed by the debug form, which is
    // exact for these values and needs no Hash impl on CssValue.
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut total = 0usize;
    for rule in &sheet.rules {
        for decl in &rule.declarations {
            total += 1;
            *seen.entry(format!("{:?}", decl.value)).or_default() += 1;
        }
    }
    let repeated: usize = seen.values().filter(|n| **n > 1).map(|n| *n - 1).sum();
    println!(
        "declared values: {total} total, {} distinct, {repeated} are a repeat of one already seen ({:.0}%)",
        seen.len(),
        100.0 * repeated as f64 / total as f64,
    );
    let mut top: Vec<_> = seen.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (value, n) in top.into_iter().take(5) {
        println!("    {n:>6} x {}", &value[..value.len().min(70)]);
    }

    // How much of the sheet is capacity nothing ever filled? A Vec grows by doubling, and a
    // parsed sheet is never appended to again.
    let rules_cap = sheet.rules.capacity();
    let mut sel_len = 0usize;
    let mut sel_cap = 0usize;
    let mut decl_len = 0usize;
    let mut decl_cap = 0usize;
    for rule in &sheet.rules {
        sel_len += rule.selectors.len();
        sel_cap += rule.selectors.capacity();
        decl_len += rule.declarations.len();
        decl_cap += rule.declarations.capacity();
    }
    println!(
        "rules      {} used of {} slots ({} wasted)",
        sheet.rules.len(),
        rules_cap,
        mb((rules_cap - sheet.rules.len()) * size_of::<gosub_css3::stylesheet::CssRule>())
    );
    println!(
        "selectors  {sel_len} used of {sel_cap} slots ({} wasted)",
        mb((sel_cap - sel_len) * size_of::<gosub_css3::stylesheet::CssSelector>())
    );
    println!(
        "decls      {decl_len} used of {decl_cap} slots ({} wasted)",
        mb((decl_cap - decl_len) * size_of::<gosub_css3::stylesheet::CssDeclaration>())
    );

    drop(sheet);
    println!("after dropping the stylesheet: {}", mb(resident()));
}
