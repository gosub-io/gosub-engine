//! Runs the covered WPT suites through the `gosub-wpt` binary and compares the result
//! against the committed expectations.
//!
//! Point `WPT_ROOT` at a web-platform-tests checkout on the commit in
//! `tests/wpt/wpt-commit.txt`. Without `WPT_ROOT` the test skips, so a normal `cargo test`
//! needs no checkout.
//!
//! When behaviour improves the expectations go stale and this fails with UNEXPECTED PASS.
//! That is the point - regenerate and commit the diff:
//!
//! ```text
//! cargo run --release -p gosub-wpt -- "$WPT_ROOT" --write-expectations \
//!     $(cd "$WPT_ROOT" && find dom/events html/dom -name '*.html' | sort) \
//!     > tests/wpt/expectations.txt
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative)
}

#[test]
fn suites_match_the_expectations() {
    let Ok(wpt_root) = std::env::var("WPT_ROOT") else {
        eprintln!("WPT_ROOT is not set, skipping the WPT conformance run");
        return;
    };
    let expectations = workspace_file("tests/wpt/expectations.txt");
    assert!(
        expectations.exists(),
        "missing {} - the expectations file lists the suites to run",
        expectations.display()
    );

    let output = Command::new(env!("CARGO_BIN_EXE_gosub-wpt"))
        .arg(&wpt_root)
        .arg("--all")
        .arg("--expect")
        .arg(&expectations)
        .output()
        .expect("running gosub-wpt");

    let report = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        // Only the lines that disagree with the expectations, so the failure is readable.
        let interesting: Vec<&str> = report
            .lines()
            .filter(|line| {
                line.contains("UNEXPECTED")
                    || line.trim_start().starts_with("FAIL ")
                    || line.trim_start().starts_with("TIMEOUT ")
                    || line.trim_start().starts_with("NOTRUN ")
                    || line.contains(": ERROR ")
            })
            .collect();
        panic!(
            "WPT results no longer match tests/wpt/expectations.txt:\n{}",
            interesting.join("\n")
        );
    }
}
