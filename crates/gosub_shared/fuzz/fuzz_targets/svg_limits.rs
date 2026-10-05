//! Differential fuzz target for `gosub_shared::svg_limits::xml_exceeds_limits`.
//!
//! The scanner estimates nesting depth from raw bytes without parsing. This feeds the same
//! document to it and to the real parser, and panics if the scanner reads *shallower* than the
//! parser builds - the only disagreement that matters, since it is the bypass the guard exists
//! to prevent. Two such bypasses were found by hand review of PR #1229; this is the mechanical
//! version of that review. See `tests/svg_limits_differential.rs` for the fixed-case form.
//!
//! The parser's `nodes_limit` keeps *it* from overflowing on the deep inputs the fuzzer will
//! find, without changing what the scanner is being checked against: a document the parser
//! refuses is never parsed, so the scanner's answer for it is moot.
#![no_main]

use gosub_shared::svg_limits::{xml_exceeds_limits, XmlLimit};
use libfuzzer_sys::fuzz_target;

/// Small enough that the parser survives any document under `NODES`, large enough that the
/// fuzzer can reach it with real nesting.
const LIMIT: usize = 24;
const NODES: u32 = 4096;

fn real_depth(xml: &str) -> Option<usize> {
    let opt = roxmltree::ParsingOptions {
        allow_dtd: true,
        nodes_limit: NODES,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(xml, opt).ok()?;
    doc.descendants()
        .filter(roxmltree::Node::is_element)
        .map(|n| n.ancestors().filter(roxmltree::Node::is_element).count())
        .max()
        .or(Some(0))
}

fuzz_target!(|data: &[u8]| {
    let Ok(xml) = std::str::from_utf8(data) else { return };
    let scanned = xml_exceeds_limits(data, LIMIT);
    let Some(real) = real_depth(xml) else { return };

    // Entities are refused by policy rather than measured; a refusal is never a bypass.
    if scanned.is_none() && real > LIMIT {
        panic!("scanner accepted a document the parser nests {real} deep (limit {LIMIT})");
    }
    // The other direction is compatibility, not security, but it is currently never the case
    // for well-formed input and losing that silently would be a regression too.
    if scanned == Some(XmlLimit::Depth) && real <= LIMIT {
        panic!("scanner refused a document the parser nests only {real} deep (limit {LIMIT})");
    }
});
