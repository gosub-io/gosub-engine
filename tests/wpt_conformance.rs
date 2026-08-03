//! Runs the WPT conformance suites for the gosub_jsapi web APIs through the
//! wpt-run binary.
//!
//! Needs a checkout of <https://github.com/web-platform-tests/wpt> — point
//! `WPT_ROOT` at it (a sparse checkout of the directories below plus
//! `resources/` and `common/` is enough; see tests/wpt/wpt-commit.txt for the
//! commit CI pins). Without `WPT_ROOT` the test skips, so a plain
//! `cargo nextest run` stays green.

use std::path::PathBuf;
use std::process::Command;

/// The validated suites. Additions land here together with any new entries in
/// tests/wpt/expected-failures.txt. Deliberately not a directory glob: a wpt
/// update must not silently change what CI covers.
const SUITES: &[&str] = &[
    // atob/btoa
    "html/webappapis/atob/base64.any.js",
    // console
    "console/console-is-a-namespace.any.js",
    "console/console-label-conversion.any.js",
    "console/console-log-large-array.any.js",
    "console/console-log-symbol.any.js",
    "console/console-namespace-object-class-string.any.js",
    "console/console-tests-historical.any.js",
    // TextEncoder/TextDecoder
    "encoding/api-basics.any.js",
    "encoding/api-invalid-label.any.js",
    "encoding/api-replacement-encodings.any.js",
    "encoding/api-surrogates-utf8.any.js",
    "encoding/iso-2022-jp-decoder.any.js",
    "encoding/textdecoder-arguments.any.js",
    "encoding/textdecoder-byte-order-marks.any.js",
    "encoding/textdecoder-copy.any.js",
    "encoding/textdecoder-eof.any.js",
    "encoding/textdecoder-fatal.any.js",
    "encoding/textdecoder-fatal-single-byte.any.js",
    "encoding/textdecoder-fatal-streaming.any.js",
    "encoding/textdecoder-ignorebom.any.js",
    "encoding/textdecoder-labels.any.js",
    "encoding/textdecoder-mistakes.any.js",
    "encoding/textdecoder-streaming.any.js",
    "encoding/textdecoder-utf16-surrogates.any.js",
    "encoding/textencoder-constructor-non-utf.any.js",
    "encoding/textencoder-utf16-surrogates.any.js",
    // URL/URLSearchParams
    "url/historical.any.js",
    "url/IdnaTestV2.any.js",
    "url/IdnaTestV2-removed.any.js",
    "url/url-constructor.any.js",
    "url/url-origin.any.js",
    "url/url-searchparams.any.js",
    "url/url-setters.any.js",
    "url/url-setters-stripping.any.js",
    "url/url-statics-canparse.any.js",
    "url/url-statics-parse.any.js",
    "url/url-tojson.any.js",
    "url/urlencoded-parser.any.js",
    "url/urlsearchparams-append.any.js",
    "url/urlsearchparams-constructor.any.js",
    "url/urlsearchparams-delete.any.js",
    "url/urlsearchparams-foreach.any.js",
    "url/urlsearchparams-get.any.js",
    "url/urlsearchparams-getall.any.js",
    "url/urlsearchparams-has.any.js",
    "url/urlsearchparams-set.any.js",
    "url/urlsearchparams-size.any.js",
    "url/urlsearchparams-sort.any.js",
    "url/urlsearchparams-stringifier.any.js",
    // Headers
    "fetch/api/headers/header-setcookie.any.js",
    "fetch/api/headers/header-values.any.js",
    "fetch/api/headers/header-values-normalize.any.js",
    "fetch/api/headers/headers-basic.any.js",
    "fetch/api/headers/headers-casing.any.js",
    "fetch/api/headers/headers-combine.any.js",
    "fetch/api/headers/headers-errors.any.js",
    "fetch/api/headers/headers-forbidden-override.any.js",
    "fetch/api/headers/headers-no-cors.any.js",
    "fetch/api/headers/headers-normalize.any.js",
    "fetch/api/headers/headers-record.any.js",
    "fetch/api/headers/headers-structure.any.js",
    // Event/EventTarget + AbortController/AbortSignal
    "dom/events/AddEventListenerOptions-once.any.js",
    "dom/events/AddEventListenerOptions-passive.any.js",
    "dom/events/AddEventListenerOptions-signal.any.js",
    "dom/events/Event-constructors.any.js",
    "dom/events/Event-isTrusted.any.js",
    "dom/events/EventTarget-addEventListener.any.js",
    "dom/events/EventTarget-add-remove-listener.any.js",
    "dom/events/EventTarget-constructible.any.js",
    "dom/events/EventTarget-removeEventListener.any.js",
    "dom/abort/abort-signal-any.any.js",
    "dom/abort/AbortSignal.any.js",
    "dom/abort/event.any.js",
    "dom/abort/timeout.any.js",
    // Storage (localStorage/sessionStorage). The window_open/noopener/reopen
    // and cross-origin-iframe tests are omitted: they need real multi-window
    // browsing contexts, like the excluded live-server suites elsewhere.
    "webstorage/defineProperty.window.js",
    "webstorage/event_constructor.window.js",
    "webstorage/event_initstorageevent.window.js",
    "webstorage/missing_arguments.window.js",
    "webstorage/set.window.js",
    "webstorage/storage_builtins.window.js",
    "webstorage/storage_clear.window.js",
    "webstorage/storage_enumerate.window.js",
    "webstorage/storage_functions_not_overwritten.window.js",
    "webstorage/storage_getitem.window.js",
    "webstorage/storage_indexing.window.js",
    "webstorage/storage_in.window.js",
    "webstorage/storage_key_empty_string.window.js",
    "webstorage/storage_key.window.js",
    "webstorage/storage_length.window.js",
    "webstorage/storage_local_quota_independent_from_session.window.js",
    "webstorage/storage_local_setitem_quotaexceedederr.window.js",
    "webstorage/storage_removeitem.window.js",
    "webstorage/storage_session_quota_independent_from_local.window.js",
    "webstorage/storage_session_setitem_quotaexceedederr.window.js",
    "webstorage/storage_set_value_enumerate.window.js",
    "webstorage/storage_setitem.window.js",
    "webstorage/storage_string_conversion.window.js",
    "webstorage/storage_supported_property_names.window.js",
    "webstorage/symbol-props.window.js",
];

#[test]
fn wpt_conformance() {
    let Ok(root) = std::env::var("WPT_ROOT") else {
        eprintln!("wpt_conformance: SKIPPED — set WPT_ROOT to a web-platform-tests checkout to run");
        return;
    };
    let root = PathBuf::from(root);
    assert!(
        root.join("resources/testharness.js").is_file(),
        "WPT_ROOT does not look like a wpt checkout (no resources/testharness.js): {}",
        root.display()
    );

    let expectations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wpt/expected-failures.txt");
    assert!(expectations.is_file(), "missing {}", expectations.display());

    let missing: Vec<&str> = SUITES.iter().copied().filter(|s| !root.join(s).is_file()).collect();
    assert!(
        missing.is_empty(),
        "wpt checkout is missing suites (extend the sparse checkout): {missing:?}"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_wpt-run"))
        .arg("--expect")
        .arg(&expectations)
        .arg(&root)
        .args(SUITES.iter().map(|s| root.join(s)))
        .output()
        .expect("failed to spawn wpt-run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let summary = stdout
        .lines()
        .rev()
        .find(|l| l.starts_with("total:"))
        .unwrap_or("<no summary>");

    if output.status.success() {
        eprintln!("wpt_conformance: {summary}");
        return;
    }

    // Show what actually failed, not the thousands of passing lines
    let interesting: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            l.starts_with("FAIL")
                || l.starts_with("TIMEOUT")
                || l.starts_with("NOTRUN")
                || l.starts_with("UNEXPECTED")
                || l.starts_with("HARNESS")
                || l.starts_with("uncaught")
                || l.starts_with("total:")
        })
        .collect();
    panic!(
        "wpt-run reported unexpected results:\n{}\nstderr:\n{}",
        interesting.join("\n"),
        String::from_utf8_lossy(&output.stderr)
    );
}
