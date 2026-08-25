//! Runs web-platform-tests `testharness.js` tests against the gosub DOM.
//!
//! The page is parsed once, up front, and then every `<script>` in it is evaluated in tree
//! order - so unlike a browser, scripts see the whole document rather than only the part
//! parsed so far. There is no event loop and no navigation: `done()` is called by the driver
//! once the last script has run.
//!
//! Usage: `gosub-wpt <wpt-root> <test.html>...`

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context as _};
use clap::Parser;
use gosub_domjs::parse_document;
use gosub_interface::document::Document as _;
use gosub_shared::node::NodeId;
use rquickjs::{CatchResultExt, Context, Ctx, Function, Runtime};
use serde::Deserialize;

/// Installed after testharness.js: the shell environment completes on an explicit `done()`,
/// and results land in a global the driver reads back out.
const RESULTS_HOOK: &str = r#"
setup({ explicit_done: true });
globalThis.__wpt_results = null;
add_completion_callback(function (tests, harness_status) {
    globalThis.__wpt_results = {
        status: harness_status.status,
        message: harness_status.message == null ? null : String(harness_status.message),
        tests: tests.map(function (t) {
            return {
                name: String(t.name),
                status: t.status,
                message: t.message == null ? null : String(t.message),
            };
        }),
    };
});
"#;

#[derive(Parser)]
#[command(name = "gosub-wpt", about = "Run WPT testharness tests against the gosub DOM")]
struct Args {
    /// Root of a web-platform-tests checkout (needs at least `resources/`)
    wpt_root: PathBuf,
    /// Test files to run, either absolute or relative to the wpt root
    tests: Vec<PathBuf>,
    /// Print every subtest, not just the failures
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Deserialize)]
struct HarnessResults {
    status: u32,
    message: Option<String>,
    tests: Vec<SubtestResult>,
}

#[derive(Deserialize)]
struct SubtestResult {
    name: String,
    status: u32,
    message: Option<String>,
}

fn status_name(status: u32) -> &'static str {
    match status {
        0 => "PASS",
        1 => "FAIL",
        2 => "TIMEOUT",
        3 => "NOTRUN",
        _ => "PRECONDITION_FAILED",
    }
}

/// A `<script>` in the document: either a source path to load, or inline text.
enum Script {
    External(String),
    Inline(String),
}

fn collect_scripts(doc: &gosub_domjs::Doc) -> Vec<Script> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = doc.children(doc.root()).iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        stack.extend(doc.children(id).iter().rev());
        if doc.tag_name(id) != Some("script") {
            continue;
        }
        if let Some(src) = doc.attribute(id, "src") {
            out.push(Script::External(src.to_string()));
            continue;
        }
        let text: String = doc.children(id).iter().filter_map(|&c| doc.text_value(c)).collect();
        if !text.trim().is_empty() {
            out.push(Script::Inline(text));
        }
    }
    out
}

/// testharness.js and its reporter are loaded by the driver, not as page scripts - the
/// reporter only knows how to write results into a browser window.
fn is_harness_script(src: &str) -> bool {
    let file = src.rsplit('/').next().unwrap_or(src);
    matches!(file, "testharness.js" | "testharnessreport.js")
}

fn resolve(wpt_root: &Path, test_dir: &Path, src: &str) -> PathBuf {
    let src = src.split(['?', '#']).next().unwrap_or(src);
    match src.strip_prefix('/') {
        Some(rest) => wpt_root.join(rest),
        None => test_dir.join(src),
    }
}

/// Run queued microtasks. testharness marks itself loaded from a promise callback, so
/// nothing completes until these have run. Real timers are still missing - the async tests
/// need a fake timer queue before they can pass.
fn drain_jobs(ctx: &Ctx<'_>) {
    for _ in 0..10_000 {
        if !ctx.execute_pending_job() {
            return;
        }
    }
}

fn install_console(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let console = rquickjs::Object::new(ctx.clone())?;
    let log = Function::new(ctx.clone(), |msg: String| println!("  [console] {msg}"))?;
    console.set("log", log.clone())?;
    console.set("warn", log.clone())?;
    console.set("error", log)?;
    ctx.globals().set("console", console)
}

fn run_test(wpt_root: &Path, test_path: &Path, verbose: bool) -> anyhow::Result<bool> {
    let source = std::fs::read_to_string(test_path).with_context(|| format!("reading {}", test_path.display()))?;
    let (doc, _parse_errors) = parse_document(&source, None)?;
    let scripts = collect_scripts(&doc.borrow());

    let test_dir = test_path.parent().unwrap_or(Path::new("."));
    let harness = std::fs::read_to_string(wpt_root.join("resources/testharness.js"))
        .with_context(|| format!("reading testharness.js under {}", wpt_root.display()))?;

    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;

    let results = context.with(|ctx| -> anyhow::Result<Option<HarnessResults>> {
        install_console(&ctx)?;

        // testharness.js needs `self` to exist, but must not see `document` yet: it picks its
        // environment by looking for one, and the window environment expects a message-passing
        // browser we do not have. `document` is installed right after, still before any test runs.
        ctx.eval::<(), _>("globalThis.self = globalThis;")
            .catch(&ctx)
            .map_err(|e| anyhow::anyhow!("globals: {e}"))?;
        ctx.eval::<(), _>(harness.as_bytes())
            .catch(&ctx)
            .map_err(|e| anyhow::anyhow!("testharness.js: {e}"))?;
        ctx.eval::<(), _>(RESULTS_HOOK)
            .catch(&ctx)
            .map_err(|e| anyhow::anyhow!("results hook: {e}"))?;

        gosub_domjs::install(&ctx, doc.clone())?;
        drain_jobs(&ctx);

        for script in &scripts {
            let (label, code) = match script {
                Script::External(src) if is_harness_script(src) => continue,
                Script::External(src) => {
                    let path = resolve(wpt_root, test_dir, src);
                    let code = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
                    (src.clone(), code)
                }
                Script::Inline(code) => ("<inline>".to_string(), code.clone()),
            };
            if let Err(e) = ctx.eval::<(), _>(code.as_bytes()).catch(&ctx) {
                println!("  script {label} threw: {e}");
            }
            drain_jobs(&ctx);
        }

        ctx.eval::<(), _>("done()")
            .catch(&ctx)
            .map_err(|e| anyhow::anyhow!("done(): {e}"))?;
        drain_jobs(&ctx);

        let json: Option<String> = ctx
            .eval::<Option<String>, _>("__wpt_results === null ? null : JSON.stringify(__wpt_results)")
            .catch(&ctx)
            .map_err(|e| anyhow::anyhow!("reading results: {e}"))?;

        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    })?;

    let Some(results) = results else {
        bail!("the harness never reported: no completion callback ran");
    };

    let (mut passed, mut failed) = (0, 0);
    for test in &results.tests {
        if test.status == 0 {
            passed += 1;
        } else {
            failed += 1;
        }
        if verbose || test.status != 0 {
            let detail = test.message.as_deref().unwrap_or("");
            println!(
                "  {} {}{}",
                status_name(test.status),
                test.name,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(" - {detail}")
                }
            );
        }
    }

    let harness_ok = results.status == 0;
    if !harness_ok {
        println!(
            "  harness {}: {}",
            status_name(results.status),
            results.message.as_deref().unwrap_or("")
        );
    }
    println!("{}: {passed} passed, {failed} failed", test_path.display());
    Ok(harness_ok && failed == 0)
}

fn main() -> ExitCode {
    eprintln!(
        "{} v{} — run WPT testharness tests against the gosub DOM",
        env!("CARGO_BIN_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    let args = Args::parse();
    let mut all_ok = true;

    for test in &args.tests {
        let path = if test.is_absolute() || test.exists() {
            test.clone()
        } else {
            args.wpt_root.join(test)
        };
        match run_test(&args.wpt_root, &path, args.verbose) {
            Ok(ok) => all_ok &= ok,
            Err(e) => {
                println!("{}: ERROR {e:#}", path.display());
                all_ok = false;
            }
        }
    }

    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
