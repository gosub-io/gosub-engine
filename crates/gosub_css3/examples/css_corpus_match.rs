// This is a developer diagnostic tool, not library/production code: panicking on bad CLI
// input and using the simplest string APIs is fine here.
#![allow(clippy::panic, clippy::disallowed_methods)]

//! Real-world validation harness for the CSS value-syntax matcher.
//!
//! Runs a directory of `.css` files through the property-definition matcher and reports
//! how much real-world CSS it accepts, plus a ranked breakdown of what it rejects - the
//! rejections are the interesting output, since they point at the next matcher gaps.
//!
//! Note: normal parsing does NOT run the matcher (`ParserConfig::match_values` is inert),
//! so this drives it explicitly, exactly like the unit tests: parse each stylesheet, then
//! for every declaration look up the property definition and call `matches()`.
//!
//! Usage:
//!   cargo run --release --example css_corpus_match -- [DIR] [--limit N] [--top N] [--samples N]
//!
//! DIR defaults to ~/code/gosub/domains/css.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use gosub_css3::matcher::property_definitions::get_css_definitions;
use gosub_css3::stylesheet::CssValue;
use gosub_css3::Css3;
use gosub_interface::css3::CssOrigin;
use gosub_shared::config::ParserConfig;

/// Flatten a declaration value into the `&[CssValue]` slice the matcher expects.
fn flatten(value: &CssValue) -> Vec<CssValue> {
    match value {
        CssValue::List(v) => v.clone(),
        other => vec![other.clone()],
    }
}

/// Render a value back into a readable CSS-ish string for failure samples (the built-in
/// `Display` wraps lists in `List(...)`, which is noise here).
fn render(value: &CssValue) -> String {
    match value {
        CssValue::List(v) => {
            let mut out = String::new();
            for (i, item) in v.iter().enumerate() {
                let is_comma = matches!(item, CssValue::Comma);
                if i > 0 && !is_comma {
                    out.push(' ');
                }
                out.push_str(&render(item));
            }
            out
        }
        other => other.to_string(),
    }
}

struct PropStats {
    total: u64,
    failed: u64,
    samples: Vec<String>,
}

fn main() {
    let mut dir: Option<PathBuf> = None;
    let mut limit = usize::MAX;
    let mut top = 25usize;
    let mut samples = 4usize;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut take = |name: &str| -> usize {
            args.next()
                .unwrap_or_else(|| panic!("{name} needs a value"))
                .parse()
                .unwrap_or_else(|_| panic!("{name} needs a number"))
        };
        match arg.as_str() {
            "--limit" => limit = take("--limit"),
            "--top" => top = take("--top"),
            "--samples" => samples = take("--samples"),
            _ => dir = Some(PathBuf::from(arg)),
        }
    }

    let dir = dir.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join("code/gosub/domains/css")
    });

    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("css")))
        .collect();
    files.sort();
    files.truncate(limit);

    let defs = get_css_definitions();

    let (mut files_ok, mut files_err) = (0u64, 0u64);
    let (mut total_decls, mut custom_decls, mut known_decls, mut matched) = (0u64, 0u64, 0u64, 0u64);
    let mut unknown_props: HashMap<String, u64> = HashMap::new();
    let mut per_prop: HashMap<String, PropStats> = HashMap::new();

    let total_files = files.len();
    for (i, path) in files.iter().enumerate() {
        if i % 200 == 0 && i > 0 {
            eprintln!("  .. {i}/{total_files} files");
        }
        let Ok(bytes) = fs::read(path) else {
            files_err += 1;
            continue;
        };
        let css = String::from_utf8_lossy(&bytes);
        let config = ParserConfig {
            match_values: false,
            ignore_errors: true,
            ..Default::default()
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let Ok(sheet) = Css3::parse_str(&css, config, CssOrigin::Author, &name) else {
            files_err += 1;
            continue;
        };
        files_ok += 1;

        for rule in &sheet.rules {
            for decl in &rule.declarations {
                total_decls += 1;
                if decl.property.starts_with("--") {
                    custom_decls += 1;
                    continue;
                }
                let prop = decl.property.to_ascii_lowercase();
                let Some(def) = defs.find_property(&prop) else {
                    *unknown_props.entry(prop).or_default() += 1;
                    continue;
                };
                known_decls += 1;
                let values = flatten(&decl.value);
                let ok = def.matches(&values);
                let stats = per_prop.entry(prop).or_insert(PropStats {
                    total: 0,
                    failed: 0,
                    samples: Vec::new(),
                });
                stats.total += 1;
                if ok {
                    matched += 1;
                } else {
                    stats.failed += 1;
                    if stats.samples.len() < samples {
                        stats.samples.push(render(&decl.value));
                    }
                }
            }
        }
    }

    let pct = |n: u64, d: u64| if d == 0 { 0.0 } else { 100.0 * n as f64 / d as f64 };

    println!("\n===== CSS matcher corpus report =====");
    println!("dir: {}", dir.display());
    println!("files: {} parsed, {} failed to parse", files_ok, files_err);
    println!("declarations: {total_decls} total");
    println!(
        "  custom (--*):      {custom_decls:>8} ({:.1}%)",
        pct(custom_decls, total_decls)
    );
    let unknown_total: u64 = unknown_props.values().sum();
    println!(
        "  unknown property:  {unknown_total:>8} ({:.1}%)  [{} distinct]",
        pct(unknown_total, total_decls),
        unknown_props.len()
    );
    println!(
        "  known property:    {known_decls:>8} ({:.1}%)",
        pct(known_decls, total_decls)
    );
    println!(
        "\nMATCHER ACCURACY on known properties: {} / {} = {:.2}%",
        matched,
        known_decls,
        pct(matched, known_decls)
    );

    // Ranked failing known properties (by absolute failure count).
    let mut failing: Vec<(&String, &PropStats)> = per_prop.iter().filter(|(_, s)| s.failed > 0).collect();
    failing.sort_by(|a, b| b.1.failed.cmp(&a.1.failed).then(a.0.cmp(b.0)));
    println!("\n----- top {top} rejected known properties (property: failed/total, rate) -----");
    for (prop, s) in failing.iter().take(top) {
        println!(
            "{:>28}: {:>7}/{:<7} ({:5.1}% rejected)  e.g. {}",
            prop,
            s.failed,
            s.total,
            pct(s.failed, s.total),
            s.samples
                .iter()
                .map(|v| format!("`{v}`"))
                .collect::<Vec<_>>()
                .join("  ")
        );
    }

    // Unknown (vendor / typo / not-in-definitions) properties by frequency.
    let mut unknown: Vec<(&String, &u64)> = unknown_props.iter().collect();
    unknown.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!("\n----- top {top} unknown properties (not in definitions) -----");
    for (prop, n) in unknown.iter().take(top) {
        println!("{:>28}: {:>8}", prop, n);
    }
    println!();
}
