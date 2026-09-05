//! GHSA-c762-mxfh-vwvp: a deeply nested SVG overflowed the stack and aborted the process instead
//! of failing to decode.
//!
//! These have to run unoptimised. Tokenizer frames are ~25x larger in a debug build (the measured
//! overflow depths were 136 and 3262), so a release-only run would sail past a limit a `cargo run`
//! browser hits - don't give this crate an `opt-level` override without revisiting that.
//!
//! Each case decodes on a 2 MiB stack, which is what `std::thread` and a tokio worker give. A
//! regression aborts the test binary rather than failing an assertion.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use gosub_render_pipeline::common::media::{MediaDecoder, SvgDecoder};

const REALISTIC_STACK: usize = 2 * 1024 * 1024;

/// Decode on a default-sized stack, reporting whether the decoder accepted the bytes.
fn decode_on_a_realistic_stack(bytes: Vec<u8>) -> Result<(), String> {
    std::thread::Builder::new()
        .stack_size(REALISTIC_STACK)
        .spawn(move || SvgDecoder::new().decode(&bytes).map(|_| ()).map_err(|e| e.to_string()))
        .expect("spawn")
        .join()
        .expect("the decoder must not abort the process")
}

fn nested(depth: usize) -> Vec<u8> {
    let mut s = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">"#);
    s.push_str(&"<g>".repeat(depth));
    s.push_str(r#"<rect width="10" height="10"/>"#);
    s.push_str(&"</g>".repeat(depth));
    s.push_str("</svg>");
    s.into_bytes()
}

#[test]
fn an_ordinary_svg_still_decodes() {
    assert!(decode_on_a_realistic_stack(nested(8)).is_ok());
    let shipped = include_bytes!("../resources/not-found.svg").to_vec();
    assert!(decode_on_a_realistic_stack(shipped).is_ok());
}

#[test]
fn a_deeply_nested_svg_is_rejected() {
    // 136 is the shallowest document that aborted before the fix; 2000 is what the PoC generates.
    for depth in [136, 2000, 50_000] {
        let err = decode_on_a_realistic_stack(nested(depth)).expect_err("must be rejected");
        assert!(err.contains("deeper than"), "depth {depth}: {err}");
    }
}

#[test]
fn svgz_is_checked_after_decompression() {
    // 50k levels compress to a couple of kilobytes, so a check on the bytes as they arrive on the
    // wire would see nothing.
    use std::io::Write;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&nested(50_000)).expect("compress");
    let svgz = gz.finish().expect("compress");

    assert!(svgz.len() < nested(50_000).len() / 100, "fixture should be a bomb");
    let err = decode_on_a_realistic_stack(svgz).expect_err("must be rejected");
    assert!(err.contains("deeper than"), "{err}");
}

#[test]
fn entity_expansion_is_checked() {
    // Replacement text is re-parsed as markup: this nests 5000 deep once expanded, while nothing
    // at the top level nests at all.
    let bomb = format!(
        "<!DOCTYPE svg [<!ENTITY deep \"{}{}\">]>\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\">&deep;</svg>",
        "<g>".repeat(5000),
        "</g>".repeat(5000),
    );
    let err = decode_on_a_realistic_stack(bomb.into_bytes()).expect_err("must be rejected");
    assert!(err.contains("deeper than"), "{err}");
}
