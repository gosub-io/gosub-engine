//! Argument errors end the run with a message and exit code 2, not a panic.

use std::process::Command;

fn run(url: &str) -> std::process::Output {
    let out = std::env::temp_dir().join("gosub-screenshot-arguments-test.png");
    Command::new(env!("CARGO_BIN_EXE_gosub-screenshot"))
        .arg(url)
        .arg(&out)
        .arg("800")
        .env_remove("RUST_BACKTRACE")
        .output()
        .expect("run gosub-screenshot")
}

#[test]
fn a_url_that_does_not_parse_is_a_usage_error() {
    // An unclosed IPv6 host, a scheme with no host, a space in a host, and the same unclosed
    // host without a scheme, which gets `https://` in front first.
    for (url, shown) in [
        ("http://[::1", "http://[::1"),
        ("http://", "http://"),
        ("https://exa mple.com", "https://exa mple.com"),
        ("[::1", "https://[::1"),
    ] {
        let output = run(url);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{url}: {stderr}");
        assert!(stderr.contains(&format!("Invalid URL '{shown}': ")), "{url}: {stderr}");
        assert!(!stderr.contains("panicked"), "{url}: {stderr}");
    }
}
