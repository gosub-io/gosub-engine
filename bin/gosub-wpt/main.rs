//! Runs web-platform-tests `testharness.js` tests against the gosub DOM.
//!
//! The page is parsed once, up front, and then every `<script>` in it is evaluated in tree
//! order - so unlike a browser, scripts see the whole document rather than only the part
//! parsed so far. There is no navigation, and the event loop is a pumped virtual-time timer
//! queue: `done()` is called once the last script has run, then timers are fired until the
//! harness reports or the queue drains.
//!
//! Usage: `gosub-wpt <wpt-root> <test.html>...`

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context as _};
use clap::Parser;
use cow_utils::CowUtils;
use gosub_domjs::parse_document;
use gosub_domjs::timers::{self, TimerState, Timers};
use gosub_interface::document::Document as _;
use gosub_shared::node::NodeId;
use rquickjs::{CatchResultExt, Context, Ctx, Function, Runtime};
use serde::Deserialize;

/// Installed after testharness.js: the shell environment completes on an explicit `done()`,
/// and results land in a global the driver reads back out.
/// The report page, with `__DATA__`, `__COMMIT__` and `__DATE__` filled in at write time.
const REPORT_TEMPLATE: &str = include_str!("report.html");

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
    /// Expectations file: known failures count as expected, and a listed test that starts
    /// passing is reported as an UNEXPECTED PASS so the file stays current.
    #[arg(long)]
    expect: Option<PathBuf>,
    /// Run every file the expectations list covers, instead of naming them on the command line
    #[arg(long)]
    all: bool,
    /// Read the test paths from a file, one per line (`-` for stdin). Blank lines and lines
    /// starting with `#` are skipped. The whole corpus does not fit in a command line - 57k
    /// paths is past ARG_MAX - and splitting the run into batches to get under it would give
    /// a separate `--report` per batch, so a run that big has to take its list this way.
    #[arg(long, value_name = "FILE")]
    tests_from: Option<PathBuf>,
    /// Write an HTML overview of the run - a coverage-report view of the whole corpus.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Print a fresh expectations file to stdout instead of a report. Regenerating is how the
    /// baseline moves: run it, read the diff, commit it.
    #[arg(long)]
    write_expectations: bool,
}

/// What an expectations file records. Files are listed explicitly so that adding tests to a
/// wpt checkout cannot silently change what is covered.
#[derive(Default)]
struct Expectations {
    files: Vec<String>,
    failing: std::collections::HashSet<String>,
    erroring: std::collections::HashSet<String>,
    /// Files whose harness itself does not finish cleanly - it timed out, or aborted on an
    /// uncaught exception. Separate from a subtest failing.
    unclean: std::collections::HashSet<String>,
}

impl Expectations {
    fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut out = Expectations::default();
        for line in text.lines() {
            // No trimming: a subtest name may legitimately end in a space.
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match line.split_once(' ') {
                Some(("FILE", rest)) => out.files.push(rest.to_string()),
                Some(("ERROR", rest)) => {
                    // A FILE record is written alongside, so only note that it errors -
                    // pushing here too would run the suite twice under --all.
                    out.erroring.insert(rest.to_string());
                }
                Some(("FAIL", rest)) => {
                    out.failing.insert(rest.to_string());
                }
                Some(("HARNESS", rest)) => {
                    out.unclean.insert(rest.to_string());
                }
                _ => bail!("unrecognised expectation line: {line:?}"),
            }
        }
        Ok(out)
    }
}

/// Escape the control characters that would otherwise break the one-record-per-line format.
fn escape(name: &str) -> String {
    name.cow_replace('\\', "\\\\")
        .cow_replace('\n', "\\n")
        .cow_replace('\r', "\\r")
        .cow_replace('\t', "\\t")
        .into_owned()
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

/// How many timer callbacks one test may fire before the driver gives up: a `setInterval`
/// that nothing clears would otherwise run forever.
const TIMER_BUDGET: usize = 100_000;

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
    let log = Function::new(ctx.clone(), |msg: String| eprintln!("  [console] {msg}"))?;
    console.set("log", log.clone())?;
    console.set("warn", log.clone())?;
    console.set("error", log)?;
    ctx.globals().set("console", console)
}

/// The file's key in an expectations file: its path relative to the wpt root.
fn expectation_key(wpt_root: &Path, test_path: &Path) -> String {
    test_path
        .strip_prefix(wpt_root)
        .unwrap_or(test_path)
        .to_string_lossy()
        .cow_replace('\\', "/")
        .into_owned()
}

/// What one suite did, in absolute terms: known failures still count as failures here, so
/// the report shows the corpus as it is rather than as the expectations describe it.
struct Outcome {
    ok: bool,
    pass: u32,
    fail: u32,
    harness: bool,
}

fn run_test(
    wpt_root: &Path,
    test_path: &Path,
    verbose: bool,
    expect: &Expectations,
    record: bool,
) -> anyhow::Result<Outcome> {
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

        let timers: Timers = std::rc::Rc::new(std::cell::RefCell::new(TimerState::default()));
        gosub_domjs::install(&ctx, doc.clone(), &timers)?;
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
                eprintln!("  script {label} threw: {e}");
            }
            drain_jobs(&ctx);
        }

        ctx.eval::<(), _>("done()")
            .catch(&ctx)
            .map_err(|e| anyhow::anyhow!("done(): {e}"))?;
        drain_jobs(&ctx);

        // Async tests finish from a timer callback, so keep pumping until the harness
        // reports or nothing is left to fire.
        for _ in 0..TIMER_BUDGET {
            if ctx.eval::<bool, _>("__wpt_results !== null").unwrap_or(false) {
                break;
            }
            if !timers::run_next(&ctx, &timers)? {
                break;
            }
        }

        // An async test whose event never arrives would otherwise hang forever: the shell
        // environment has no default timeout, so nothing marks it. Once the queue is dry the
        // driver plays the part of the timeout the browser would have applied.
        if !ctx.eval::<bool, _>("__wpt_results !== null").unwrap_or(false) {
            ctx.eval::<(), _>("timeout()")
                .catch(&ctx)
                .map_err(|e| anyhow::anyhow!("timeout(): {e}"))?;
            drain_jobs(&ctx);
        }

        let json: Option<String> = ctx
            .eval::<Option<String>, _>("__wpt_results === null ? null : JSON.stringify(__wpt_results)")
            .catch(&ctx)
            .map_err(|e| anyhow::anyhow!("reading results: {e}"))?;

        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    })?;

    let Some(results) = results else {
        bail!("the harness never reported: no completion callback ran");
    };

    let key = expectation_key(wpt_root, test_path);
    if record {
        println!("FILE {key}");
        if results.status != 0 {
            println!("HARNESS {key}");
        }
        for test in &results.tests {
            if test.status != 0 {
                println!("FAIL {key} :: {}", escape(&test.name));
            }
        }
        return Ok(Outcome {
            ok: true,
            pass: results.tests.iter().filter(|t| t.status == 0).count() as u32,
            fail: results.tests.iter().filter(|t| t.status != 0).count() as u32,
            harness: results.status != 0,
        });
    }
    let (mut passed, mut failed, mut expected, mut unexpected_pass) = (0, 0, 0, 0);
    for test in &results.tests {
        let known = expect.failing.contains(&format!("{key} :: {}", escape(&test.name)));
        match (test.status == 0, known) {
            (true, false) => passed += 1,
            (true, true) => {
                unexpected_pass += 1;
                println!("  UNEXPECTED PASS {}", test.name);
            }
            (false, true) => expected += 1,
            (false, false) => {
                failed += 1;
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
        if verbose && test.status == 0 && !known {
            println!("  PASS {}", test.name);
        }
    }

    let harness_ok = results.status == 0 || expect.unclean.contains(&key);
    if results.status != 0 {
        println!(
            "  harness {}: {}",
            status_name(results.status),
            results.message.as_deref().unwrap_or("")
        );
    }
    let known = if expected > 0 {
        format!(", {expected} known")
    } else {
        String::new()
    };
    println!("{}: {passed} passed, {failed} failed{known}", test_path.display());
    Ok(Outcome {
        ok: harness_ok && failed == 0 && unexpected_pass == 0,
        pass: passed,
        fail: failed + expected,
        harness: results.status != 0,
    })
}

/// One suite's line in the report.
#[derive(serde::Serialize)]
struct Row {
    path: String,
    pass: u32,
    fail: u32,
    harness: bool,
    error: bool,
}

/// What the run covered, for the page subtitle: the distinct two-segment prefixes of the
/// suites in it, so a run over `dom/events` and `html/dom` says so rather than naming
/// whichever directory the template was written against.
fn scope_of(rows: &[Row]) -> String {
    let mut prefixes: Vec<String> = rows
        .iter()
        .map(|row| {
            let mut parts = row.path.split('/');
            match (parts.next(), parts.next()) {
                (Some(a), Some(b)) => format!("{a}/{b}"),
                (Some(a), None) => a.to_string(),
                _ => String::new(),
            }
        })
        .collect();
    prefixes.sort_unstable();
    prefixes.dedup();
    // A long tail of directories would push the header around; past a handful just count.
    match prefixes.len() {
        0 => "nothing".to_string(),
        1..=4 => prefixes.join(", "),
        n => format!("{} directories", n),
    }
}

/// Write the overview page: the template with this run's rows inlined.
fn write_report(path: &Path, rows: &[Row], wpt_root: &Path) -> anyhow::Result<()> {
    let commit = std::fs::read_to_string(wpt_root.join(".git/HEAD"))
        .ok()
        .and_then(|head| {
            let head = head.trim().to_string();
            match head.strip_prefix("ref: ") {
                Some(reference) => std::fs::read_to_string(wpt_root.join(".git").join(reference)).ok(),
                None => Some(head),
            }
        })
        .map(|sha| sha.trim().chars().take(10).collect::<String>())
        .unwrap_or_else(|| "unknown".to_string());

    let data = serde_json::to_string(&serde_json::json!({ "files": rows }))?;
    let page = REPORT_TEMPLATE
        .cow_replace("__DATA__", &data)
        .cow_replace("__COMMIT__", &commit)
        .cow_replace("__SCOPE__", &scope_of(rows))
        .cow_replace("__DATE__", &today())
        .into_owned();
    std::fs::write(path, page).with_context(|| format!("writing {}", path.display()))?;
    println!("report written to {}", path.display());
    Ok(())
}

/// The date, from the filesystem rather than a clock crate: the report only needs to say
/// roughly when it was made.
fn today() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (now / 86_400) as i64;
    let (mut year, mut remaining) = (1970, days);
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let length = if leap { 366 } else { 365 };
        if remaining < length {
            break;
        }
        remaining -= length;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let months = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 0;
    while remaining >= months[month] {
        remaining -= months[month];
        month += 1;
    }
    format!("{year:04}-{:02}-{:02}", month + 1, remaining + 1)
}

/// Run one file, turning a hard error into a pass when the expectations say it cannot run.
fn run_or_expect_error(
    wpt_root: &Path,
    path: &Path,
    verbose: bool,
    expect: &Expectations,
    record: bool,
) -> (bool, Row) {
    let key = expectation_key(wpt_root, path);
    let row = |pass, fail, harness, error| Row {
        path: key.clone(),
        pass,
        fail,
        harness,
        error,
    };
    match run_test(wpt_root, path, verbose, expect, record) {
        Ok(outcome) => {
            if !record && expect.erroring.contains(&key) {
                println!("{}: UNEXPECTED RUN (listed as ERROR)", path.display());
                return (false, row(outcome.pass, outcome.fail, outcome.harness, false));
            }
            (outcome.ok, row(outcome.pass, outcome.fail, outcome.harness, false))
        }
        Err(e) => {
            if record {
                // FILE as well as ERROR: the FILE records are what names the covered set, so
                // a suite that only ever errors still has to appear among them. Without it,
                // regenerating from the file's own FILE lines drops the suite silently and
                // coverage shrinks a little every time.
                println!("FILE {key}");
                println!("ERROR {key}");
                return (true, row(0, 0, false, true));
            }
            if expect.erroring.contains(&key) {
                println!("{}: known ERROR ({e:#})", path.display());
                return (true, row(0, 0, false, true));
            }
            println!("{}: ERROR {e:#}", path.display());
            (false, row(0, 0, false, true))
        }
    }
}

/// Read test paths from a file, one per line, or from stdin when the path is `-`.
///
/// Blank lines and `#` comments are skipped so a list can carry a note about what it selects,
/// but nothing else is trimmed: a path may legitimately contain leading or trailing spaces.
fn read_test_list(path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    use std::io::Read as _;

    let text = if path == Path::new("-") {
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer).context("reading stdin")?;
        buffer
    } else {
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    };

    Ok(text
        .lines()
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(PathBuf::from)
        .collect())
}

fn main() -> ExitCode {
    eprintln!(
        "{} v{} — run WPT testharness tests against the gosub DOM",
        env!("CARGO_BIN_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    let args = Args::parse();
    let expect = match args.expect.as_deref().map(Expectations::load).transpose() {
        Ok(expect) => expect.unwrap_or_default(),
        Err(e) => {
            println!("could not read expectations: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    let mut tests: Vec<PathBuf> = if args.all {
        expect.files.iter().map(PathBuf::from).collect()
    } else {
        args.tests.clone()
    };
    if let Some(list) = args.tests_from.as_deref() {
        match read_test_list(list) {
            Ok(from_file) => tests.extend(from_file),
            Err(e) => {
                println!("could not read the test list: {e:#}");
                return ExitCode::FAILURE;
            }
        }
    }
    if tests.is_empty() {
        println!("no tests given (pass paths, --tests-from a file, or --all with --expect)");
        return ExitCode::FAILURE;
    }

    let mut all_ok = true;
    let mut rows = Vec::with_capacity(tests.len());
    for test in &tests {
        // A relative path means "inside the wpt root", so try there first. Taking it as
        // given whenever it happened to exist in the working directory let a same-named
        // local file shadow the real suite, and its scripts would then resolve against the
        // wrong directory - only to fall back to the cwd when the root has no such file.
        let path = if test.is_absolute() {
            test.clone()
        } else {
            let in_root = args.wpt_root.join(test);
            if in_root.exists() {
                in_root
            } else {
                test.clone()
            }
        };
        let (ok, row) = run_or_expect_error(&args.wpt_root, &path, args.verbose, &expect, args.write_expectations);
        all_ok &= ok;
        rows.push(row);
    }

    if let Some(report) = args.report.as_deref() {
        if let Err(e) = write_report(report, &rows, &args.wpt_root) {
            println!("could not write the report: {e:#}");
            return ExitCode::FAILURE;
        }
    }

    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
