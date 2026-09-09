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

/// Written at the top of every `--write-expectations` run, so that a baseline explains its own
/// format. Only what is true of every expectations file belongs here: anything specific to one
/// component (why its rate is what it is, what would move it) goes in `docs/wpt.md`, which a
/// regeneration cannot overwrite.
const EXPECTATIONS_HEADER: &str = "\
# What the engine passes today, written by `gosub-wpt --write-expectations`. Regenerate rather
# than edit:
#
#   gosub-wpt <wpt-root> <paths...> --write-expectations > this-file
#
# This records what PASSES, not what fails. The engine fails most of the corpus, so the pass
# list is a fifteenth the size of the failure list would be, and \"these subtests pass and must
# keep passing\" is the property worth committing. A fix then shows up as added lines.
#
# One record per line:
#   FILE    <path>            a suite this baseline covers. Listed explicitly, so adding files
#                             to a wpt checkout cannot silently change what is covered.
#   PASS    <path> :: <name>  a subtest that passes and must keep passing. Control characters
#                             in the name are escaped \\n \\r \\t, and the rest of the C0 range
#                             as \\xNN.
#   HARNESS <path>            a suite whose harness does not finish cleanly (timed out, or
#                             aborted) - separate from any individual subtest failing.
#   ERROR   <path>            a suite that cannot run at all here, usually a support file
#                             outside the sparse checkout.
#   CRASH   <path>            a suite that panics the engine. The most serious record here: a
#                             bug a real page could reach, to be fixed rather than lived with.
#
# A listed subtest that stops passing is a REGRESSION and fails the run, as is one the suite no
# longer reports at all (MISSING - renamed upstream, or its suite died before reaching it). A
# subtest that starts passing is an UNEXPECTED PASS and also fails the run, so improving the
# engine forces this file to be regenerated and it always says what the engine actually does. A
# subtest that is failing and is not listed is the ordinary state of most of the corpus, and is
# reported by none of them.
#
# See docs/wpt.md for how to run a component and how to pick something to fix.
";

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
    /// Test files to run, either absolute or relative to the wpt root. A directory runs every
    /// testharness suite underneath it, so a whole component can be named at once.
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
    /// List the suites worth picking up instead of the per-suite results: the ones that crash
    /// the engine, then the ones that partly pass, most nearly-working first.
    #[arg(long)]
    shortlist: bool,
}

/// What an expectations file records. Files are listed explicitly so that adding tests to a
/// wpt checkout cannot silently change what is covered.
#[derive(Default)]
struct Expectations {
    /// Whether a file was actually loaded. Without one every subtest is "not recorded as
    /// passing", which is the same shape as a known failure - so a plain run would fall silent
    /// about the very failures it exists to show. This tells the two apart.
    loaded: bool,
    files: Vec<String>,
    /// Subtests recorded as passing, indexed by the suite they belong to. The file lists passes
    /// rather than failures because the engine fails most of the corpus: at 2,348 of 50,310 the
    /// pass list is a fifteenth the size, and "these pass and must keep passing" is the property
    /// worth committing. It also makes a fix read as added lines rather than as thousands
    /// vanishing from a 48k file.
    ///
    /// Grouped per suite rather than kept as one flat set so that a run can tell which records
    /// went unmatched. A subtest that stops being reported at all - renamed upstream, or its
    /// suite dying before it runs - would otherwise satisfy the baseline by absence.
    passing: std::collections::HashMap<String, std::collections::HashSet<String>>,
    erroring: std::collections::HashSet<String>,
    /// Files whose harness itself does not finish cleanly - it timed out, or aborted on an
    /// uncaught exception. Separate from a subtest failing.
    unclean: std::collections::HashSet<String>,
    /// Files that panic the engine. Kept apart from `erroring` so that a crash cannot be
    /// absorbed into the baseline as though it were a missing support file.
    crashing: std::collections::HashSet<String>,
}

impl Expectations {
    fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut out = Expectations {
            loaded: true,
            ..Expectations::default()
        };
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
                Some(("PASS", rest)) => {
                    let (file, name) = rest
                        .split_once(" :: ")
                        .with_context(|| format!("PASS record has no ' :: ' separator: {rest:?}"))?;
                    out.passing
                        .entry(file.to_string())
                        .or_default()
                        .insert(name.to_string());
                }
                Some(("HARNESS", rest)) => {
                    out.unclean.insert(rest.to_string());
                }
                Some(("CRASH", rest)) => {
                    // Like ERROR, a FILE record is written alongside, so only note the crash.
                    out.crashing.insert(rest.to_string());
                }
                _ => bail!("unrecognised expectation line: {line:?}"),
            }
        }
        Ok(out)
    }
}

/// Escape the control characters that would otherwise break the one-record-per-line format.
///
/// The named three are not enough on their own: `css/css-syntax` walks the whole C0 range
/// looking for what a parser must treat as whitespace, so its subtest names carry raw
/// control bytes, and writing those through left the committed baseline a file `grep` and
/// `git diff` both refuse to treat as text. The rest become `\xNN`.
///
/// One-way, deliberately - nothing ever reads a name back into its original form. Both the
/// baseline and the name a run compares against it go through here, so the two agree without
/// the file having to be unescapable.
fn escape(name: &str) -> String {
    let escaped = name
        .cow_replace('\\', "\\\\")
        .cow_replace('\n', "\\n")
        .cow_replace('\r', "\\r")
        .cow_replace('\t', "\\t")
        .into_owned();
    if !escaped.chars().any(|ch| ch.is_control()) {
        return escaped;
    }
    escaped
        .chars()
        .map(|ch| {
            if ch.is_control() {
                format!("\\x{:02x}", ch as u32)
            } else {
                ch.to_string()
            }
        })
        .collect()
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

/// Evaluate a classic script the way a page does, in sloppy mode.
///
/// rquickjs defaults to `strict: true`, which is the wrong default for wpt. Its tests are
/// classic scripts, not modules, so `onload = ...` on an undeclared name creates a global and
/// `for (unitEntry in units)` needs no declaration. Forced into strict mode both raise a
/// ReferenceError, and the suite dies on a line every browser runs without complaint - a
/// failure that says nothing at all about the engine.
fn eval_classic(ctx: &Ctx<'_>, code: &[u8]) -> rquickjs::Result<()> {
    // `EvalOptions` is #[non_exhaustive], so the one field is changed on a default rather than
    // the struct being written out.
    let mut options = rquickjs::context::EvalOptions::default();
    options.strict = false;
    ctx.eval_with_options(code, options)
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

fn install_console(ctx: &Ctx<'_>, mode: Reporting) -> rquickjs::Result<()> {
    let console = rquickjs::Object::new(ctx.clone())?;
    // A test's own logging is a diagnostic like any other, so `--shortlist` drops it: that run
    // is a listing, and a stray `console.log` in the middle of the table reads as part of it.
    let quiet = mode.is_quiet();
    let log = Function::new(ctx.clone(), move |msg: String| {
        if !quiet {
            eprintln!("  [console] {msg}");
        }
    })?;
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

/// What a run prints as it goes. These are modes rather than independent flags because they
/// are mutually exclusive: `--write-expectations` has no use for `-v`, and `--shortlist`
/// prints its own view once the whole run is in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reporting {
    /// Per-suite results and per-subtest failures. The default.
    Normal,
    /// The same, plus every passing subtest by name (`-v`).
    Verbose,
    /// Expectation records on stdout and nothing else (`--write-expectations`).
    Record,
    /// Nothing per suite; the caller reports at the end (`--shortlist`).
    Quiet,
}

impl Reporting {
    fn is_record(self) -> bool {
        self == Reporting::Record
    }

    /// Whether the running commentary is suppressed. `Record` counts: its records are the
    /// file being written, and a stray human line would land in the committed baseline.
    fn is_quiet(self) -> bool {
        matches!(self, Reporting::Record | Reporting::Quiet)
    }

    fn is_verbose(self) -> bool {
        self == Reporting::Verbose
    }
}

fn run_test(wpt_root: &Path, test_path: &Path, expect: &Expectations, mode: Reporting) -> anyhow::Result<Outcome> {
    let source = std::fs::read_to_string(test_path).with_context(|| format!("reading {}", test_path.display()))?;
    let (doc, _parse_errors) = parse_document(&source, None)?;
    let scripts = collect_scripts(&doc.borrow());

    let test_dir = test_path.parent().unwrap_or(Path::new("."));
    let harness = std::fs::read_to_string(wpt_root.join("resources/testharness.js"))
        .with_context(|| format!("reading testharness.js under {}", wpt_root.display()))?;

    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;

    let results = context.with(|ctx| -> anyhow::Result<Option<HarnessResults>> {
        install_console(&ctx, mode)?;

        // testharness.js needs `self` to exist, but must not see `document` yet: it picks its
        // environment by looking for one, and the window environment expects a message-passing
        // browser we do not have. `document` is installed right after, still before any test runs.
        ctx.eval::<(), _>("globalThis.self = globalThis;")
            .catch(&ctx)
            .map_err(|e| anyhow::anyhow!("globals: {e}"))?;
        eval_classic(&ctx, harness.as_bytes())
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
            if let Err(e) = eval_classic(&ctx, code.as_bytes()).catch(&ctx) {
                // One line by default. A corpus run trips hundreds of these - mostly a Web API
                // the engine has not got yet - and a stack under every one buries the results
                // they are meant to annotate. `-v` keeps the full trace for investigating one,
                // and `--shortlist` wants none of it: that run is a listing, not a diagnosis.
                let text = e.to_string();
                if mode.is_verbose() {
                    eprintln!("  script {label} threw: {text}");
                } else if !mode.is_quiet() {
                    eprintln!("  script {label} threw: {}", text.lines().next().unwrap_or("").trim());
                }
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
    if mode.is_record() {
        println!("FILE {key}");
        if results.status != 0 {
            println!("HARNESS {key}");
        }
        for test in &results.tests {
            if test.status == 0 {
                println!("PASS {key} :: {}", escape(&test.name));
            }
        }
        return Ok(Outcome {
            ok: true,
            pass: results.tests.iter().filter(|t| t.status == 0).count() as u32,
            fail: results.tests.iter().filter(|t| t.status != 0).count() as u32,
            harness: results.status != 0,
        });
    }
    // `regressed` is a subtest the baseline records as passing that no longer does - the one
    // outcome this tool exists to catch. `known_fail` is a subtest that was already failing and
    // still is, which is the ordinary state of most of the corpus and says nothing new.
    let (mut passed, mut regressed, mut known_fail, mut unexpected_pass) = (0, 0, 0, 0);
    let empty = std::collections::HashSet::new();
    let recorded_here = expect.passing.get(&key).unwrap_or(&empty);
    // Which of this suite's records the run actually saw. What is left over at the end is a
    // subtest the baseline expects to pass that the suite no longer reports at all.
    let mut unseen: std::collections::HashSet<&str> = recorded_here.iter().map(String::as_str).collect();
    for test in &results.tests {
        let escaped = escape(&test.name);
        let recorded = recorded_here.contains(&escaped);
        unseen.remove(escaped.as_str());
        let detail = || match test.message.as_deref().unwrap_or("") {
            "" => String::new(),
            message => format!(" - {message}"),
        };
        match (test.status == 0, recorded) {
            (true, true) => passed += 1,
            (true, false) => {
                unexpected_pass += 1;
                // Only news against a baseline. Without one, a pass is just a pass.
                if expect.loaded && !mode.is_quiet() {
                    println!("  UNEXPECTED PASS {}", test.name);
                }
            }
            (false, true) => {
                regressed += 1;
                if !mode.is_quiet() {
                    println!("  REGRESSION {} {}{}", status_name(test.status), test.name, detail());
                }
            }
            (false, false) => {
                known_fail += 1;
                // With a baseline loaded this is the recorded state and printing it would bury
                // the regressions in tens of thousands of lines. Without one, it is the whole
                // point of the run.
                if !expect.loaded && !mode.is_quiet() {
                    println!("  {} {}{}", status_name(test.status), test.name, detail());
                }
            }
        }
        if mode.is_verbose() && test.status == 0 && !(expect.loaded && recorded) {
            println!("  PASS {}", test.name);
        }
    }

    // A record the suite never reported. Counted with the regressions because it is one: the
    // baseline says this passes, and the run cannot show that it does.
    let missing = unseen.len() as u32;
    regressed += missing;
    if missing > 0 && !mode.is_quiet() {
        let mut names: Vec<&&str> = unseen.iter().collect();
        names.sort_unstable();
        for name in names {
            println!("  MISSING {name} - recorded as passing, but the suite no longer reports it");
        }
    }

    let harness_ok = results.status == 0 || expect.unclean.contains(&key);
    if results.status != 0 && !mode.is_quiet() {
        println!(
            "  harness {}: {}",
            status_name(results.status),
            results.message.as_deref().unwrap_or("")
        );
    }
    if !mode.is_quiet() {
        let moved = match (regressed, unexpected_pass) {
            (0, 0) => String::new(),
            (0, gained) => format!(", {gained} newly passing"),
            (lost, 0) => format!(", {lost} REGRESSED"),
            (lost, gained) => format!(", {lost} REGRESSED, {gained} newly passing"),
        };
        println!(
            "{}: {} passed, {} failed{moved}",
            test_path.display(),
            passed + unexpected_pass,
            regressed + known_fail
        );
    }
    Ok(Outcome {
        ok: harness_ok && regressed == 0 && unexpected_pass == 0,
        // Both counts describe the engine rather than the baseline's opinion of it: an
        // unexpected pass is still a pass, and a regression is still a failure. The moment that
        // matters most is the run right after a fix, where leaving the new passes out would
        // report the numbers as unmoved.
        pass: passed + unexpected_pass,
        fail: regressed + known_fail,
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
    /// The engine panicked while running this suite. Kept apart from `error`, which is a suite
    /// this tool cannot run (a support file outside the checkout): a panic is a bug in engine
    /// code that a real page could reach, and folding the two together would let the more
    /// serious one hide inside the baseline.
    crash: bool,
    /// The panic message, so `--shortlist` can say what crashed without the reader re-running
    /// the suite to find out.
    #[serde(skip_serializing_if = "Option::is_none")]
    crash_message: Option<String>,
}

thread_local! {
    /// The message from the most recent panic, stashed by the hook below so the runner can put
    /// it on the suite's line. The default hook is replaced rather than kept because one engine
    /// panic is typically reached by hundreds of suites in a corpus run, and a backtrace note
    /// after every one buries the report it is meant to annotate.
    static LAST_PANIC: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Route panic messages into `LAST_PANIC` instead of stderr.
fn capture_panics() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|message| (*message).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panicked".to_string());
        let detail = match info.location() {
            Some(at) => format!("{payload} (at {}:{})", at.file(), at.line()),
            None => payload,
        };
        LAST_PANIC.with(|slot| *slot.borrow_mut() = Some(detail));
    }));
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
fn run_or_expect_error(wpt_root: &Path, path: &Path, expect: &Expectations, mode: Reporting) -> (bool, Row) {
    let key = expectation_key(wpt_root, path);
    let row = |pass, fail, harness, error| Row {
        path: key.clone(),
        pass,
        fail,
        harness,
        error,
        crash: false,
        crash_message: None,
    };
    let crashed = |detail: &str| Row {
        path: key.clone(),
        pass: 0,
        fail: 0,
        harness: false,
        error: false,
        crash: true,
        crash_message: Some(detail.to_string()),
    };

    // A panic in engine code must not take the rest of the corpus with it: one bad property
    // definition would otherwise end a 309-suite run at whichever file reached it first, and
    // the report would silently describe only the part before the crash. The runtime is built
    // inside `run_test` and dropped as the panic unwinds, so the next suite starts clean.
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_test(wpt_root, path, expect, mode)));
    let outcome = match caught {
        Ok(outcome) => outcome,
        Err(_) => {
            let detail = LAST_PANIC
                .with(|slot| slot.borrow_mut().take())
                .unwrap_or_else(|| "panicked".to_string());
            if mode.is_record() {
                println!("FILE {key}");
                println!("CRASH {key}");
                return (true, crashed(&detail));
            }
            if expect.crashing.contains(&key) {
                if !mode.is_quiet() {
                    println!("{}: known CRASH ({detail})", path.display());
                }
                return (true, crashed(&detail));
            }
            if !mode.is_quiet() {
                println!("{}: CRASH {detail}", path.display());
            }
            return (false, crashed(&detail));
        }
    };
    match outcome {
        Ok(outcome) => {
            if !mode.is_record() && expect.erroring.contains(&key) {
                if !mode.is_quiet() {
                    println!("{}: UNEXPECTED RUN (listed as ERROR)", path.display());
                }
                return (false, row(outcome.pass, outcome.fail, outcome.harness, false));
            }
            (outcome.ok, row(outcome.pass, outcome.fail, outcome.harness, false))
        }
        Err(e) => {
            if mode.is_record() {
                // FILE as well as ERROR: the FILE records are what names the covered set, so
                // a suite that only ever errors still has to appear among them. Without it,
                // regenerating from the file's own FILE lines drops the suite silently and
                // coverage shrinks a little every time.
                println!("FILE {key}");
                println!("ERROR {key}");
                return (true, row(0, 0, false, true));
            }
            if expect.erroring.contains(&key) {
                if !mode.is_quiet() {
                    println!("{}: known ERROR ({e:#})", path.display());
                }
                return (true, row(0, 0, false, true));
            }
            if !mode.is_quiet() {
                println!("{}: ERROR {e:#}", path.display());
            }
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

/// Expand a directory into the testharness suites underneath it.
///
/// wpt keeps four kinds of file in one tree and only one of them means anything here: the
/// reference halves of reftests, the `conformance-checkers/` fixtures and the manual tests
/// all parse fine, report zero subtests, and cost a QuickJS context each. Selecting on the
/// harness script rather than on the path is what makes `gosub-wpt <root> css/css-values` a
/// run of 270 suites rather than of 518 files.
fn discover(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in walkdir::WalkDir::new(dir).follow_links(false) {
        let entry = entry.with_context(|| format!("walking {}", dir.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension() != Some(std::ffi::OsStr::new("html")) {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        if stem.ends_with("-ref") || stem.ends_with("-notref") {
            continue;
        }
        // Read rather than parse: the harness link is a literal `src` in the markup, and a
        // substring test over the file is far cheaper than building a document for every one
        // of the tens of thousands of files a top-level directory can hold.
        match std::fs::read_to_string(path) {
            Ok(text) if text.contains("resources/testharness.js") => found.push(path.to_path_buf()),
            // Not UTF-8, or unreadable: either way it is not a suite this can run.
            _ => continue,
        }
    }
    found.sort();
    Ok(found)
}

/// Resolve a test argument against the wpt root.
///
/// A relative path means "inside the wpt root", so try there first. Taking it as given
/// whenever it happened to exist in the working directory let a same-named local file shadow
/// the real suite, and its scripts would then resolve against the wrong directory - only to
/// fall back to the cwd when the root has no such file.
fn resolve_test_path(wpt_root: &Path, test: &Path) -> PathBuf {
    if test.is_absolute() {
        return test.to_path_buf();
    }
    let in_root = wpt_root.join(test);
    if in_root.exists() {
        in_root
    } else {
        test.to_path_buf()
    }
}

/// A ten-cell progress bar, so a rate is readable without reading the number.
fn bar(pass: u32, total: u32) -> String {
    // Round down, but never all the way to empty while anything passes: a directory at 0.4%
    // and one at a flat 0% are different news for someone looking for something to work on.
    let filled = if total == 0 {
        0
    } else {
        let tenths = ((f64::from(pass) / f64::from(total)) * 10.0).floor() as usize;
        tenths.max(usize::from(pass > 0)).min(10)
    };
    "\u{2588}".repeat(filled) + &"\u{2591}".repeat(10 - filled)
}

/// The suites worth picking up, most tractable first.
///
/// Exists so that "what should I work on" is a command rather than a name in a document. A
/// worked example in the docs stops being true the moment someone acts on it - the better the
/// documentation, the faster it rots - so the docs point here instead, and this answers against
/// the engine as it is right now.
///
/// A partly-passing suite is the tractable kind: the engine already understands the shape of
/// the thing and is wrong about a detail. One at 0/40 is usually missing a whole feature and is
/// a project, so it is left out; so is one that fully passes. Crashes come first regardless of
/// their numbers, being bugs a real page could reach rather than absent features.
fn print_shortlist(rows: &[Row]) {
    let crashes: Vec<&Row> = rows.iter().filter(|row| row.crash).collect();
    let mut partial: Vec<&Row> = rows
        .iter()
        .filter(|row| !row.crash && !row.error && row.pass > 0 && row.fail > 0)
        .collect();
    // Nearest to working first: the closer a suite is to passing, the smaller the gap left to
    // understand. Fewer remaining failures breaks the tie, so of two suites at the same rate
    // the shorter job comes first.
    partial.sort_by(|a, b| {
        let rate = |row: &Row| f64::from(row.pass) / f64::from(row.pass + row.fail);
        rate(b)
            .partial_cmp(&rate(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.fail.cmp(&b.fail))
    });

    if crashes.is_empty() && partial.is_empty() {
        println!();
        println!("  Nothing partly passing here - every suite either fully passes or fully fails.");
        println!("  Try a wider directory, or see docs/wpt.md for the larger pieces of work.");
        return;
    }

    if !crashes.is_empty() {
        println!();
        println!("  Crashes - engine code panicking on input a real page could carry. Take these first.");
        for row in &crashes {
            println!("    {}", row.path);
            if let Some(message) = &row.crash_message {
                println!("      {message}");
            }
        }
    }

    if !partial.is_empty() {
        let width = partial.iter().map(|row| row.path.len()).max().unwrap_or(0).min(70);
        println!();
        println!("  Partly passing, nearest to working first:");
        for row in &partial {
            let total = row.pass + row.fail;
            let rate = f64::from(row.pass) / f64::from(total) * 100.0;
            println!(
                "    {:>5.1}%  {:<width$}  {}/{}, {} left",
                rate, row.path, row.pass, total, row.fail
            );
        }
    }

    println!();
    println!("  Run one on its own to see the failing subtests by name, then read what passes");
    println!("  against what fails. docs/wpt-quickstart.md walks the whole loop.");
}

/// The end-of-run summary: a rollup per directory, then the totals.
///
/// Grouped by each suite's own directory rather than by a fixed prefix depth, because that is
/// the granularity work gets picked at - `css/css-values/animations` sitting at 2% while
/// `css/css-values/calc` is at 40% is the useful shape, and one `css/css-values` line
/// averaging the two together is not.
fn print_summary(rows: &[Row]) {
    use std::collections::BTreeMap;

    // BTreeMap: the directories come out in path order, which is the order they are read in.
    let mut by_dir: BTreeMap<&str, (u32, u32, u32)> = BTreeMap::new();
    for row in rows {
        let dir = row.path.rsplit_once('/').map_or(".", |(dir, _)| dir);
        let slot = by_dir.entry(dir).or_default();
        slot.0 += row.pass;
        slot.1 += row.fail;
        slot.2 += u32::from(row.error);
    }

    let width = by_dir.keys().map(|dir| dir.len()).max().unwrap_or(0).min(48);
    // The fractions are right-aligned as a column of their own, so the eye can compare two
    // directories' totals without re-reading where one number ends and the next begins.
    let counts_width = by_dir
        .values()
        .map(|(pass, fail, _)| format!("{pass}/{}", pass + fail).len())
        .max()
        .unwrap_or(0);
    println!();
    for (dir, (pass, fail, errors)) in &by_dir {
        let total = pass + fail;
        // A directory whose suites all failed to run has no subtests to take a rate over;
        // printing "0/0 0.0%" there would read as a result rather than as an absence.
        if total == 0 {
            println!("  {dir:<width$}  {}  {errors} could not run", bar(0, 0));
            continue;
        }
        let rate = f64::from(*pass) / f64::from(total) * 100.0;
        let counts = format!("{pass}/{total}");
        println!(
            "  {dir:<width$}  {}  {counts:>counts_width$}  {rate:5.1}%",
            bar(*pass, total)
        );
    }

    let files = rows.len();
    let errored = rows.iter().filter(|row| row.error).count();
    let crashed = rows.iter().filter(|row| row.crash).count();
    let clean = rows
        .iter()
        .filter(|row| !row.error && !row.crash && !row.harness && row.fail == 0)
        .count();
    let (pass, fail) = rows.iter().fold((0, 0), |(p, f), row| (p + row.pass, f + row.fail));
    println!();
    println!(
        "  {files} files: {clean} fully passing, {} with failures, {errored} could not run",
        files - clean - errored - crashed
    );
    println!("  {} subtests: {pass} passed, {fail} failed", pass + fail);
    // Last, and only when there are any: a panic is an engine bug a real page could reach, so
    // it should be the line left on screen rather than a number folded into the ones above.
    if crashed > 0 {
        println!("  {crashed} files crashed the engine (grep the run for CRASH)");
    }
}

fn main() -> ExitCode {
    eprintln!(
        "{} v{} — run WPT testharness tests against the gosub DOM",
        env!("CARGO_BIN_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    capture_panics();

    let args = Args::parse();
    // Record wins over shortlist: it is the mode that writes a file, and getting a shortlist
    // into a committed baseline would be worse than ignoring a flag.
    let mode = match (args.write_expectations, args.shortlist, args.verbose) {
        (true, _, _) => Reporting::Record,
        (false, true, _) => Reporting::Quiet,
        (false, false, true) => Reporting::Verbose,
        (false, false, false) => Reporting::Normal,
    };
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
        println!("no tests given (pass paths or directories, --tests-from a file, or --all with --expect)");
        return ExitCode::FAILURE;
    }

    // Expand directory arguments, so a whole component can be named instead of listing its
    // suites. Done up front rather than inside the run loop because the summary needs to know
    // how many suites there are before the first one runs.
    let mut paths = Vec::with_capacity(tests.len());
    for test in &tests {
        let path = resolve_test_path(&args.wpt_root, test);
        if !path.is_dir() {
            paths.push(path);
            continue;
        }
        match discover(&path) {
            Ok(found) if found.is_empty() => {
                println!("{}: no testharness.js suites under this directory", path.display());
            }
            Ok(found) => paths.extend(found),
            Err(e) => {
                println!("could not read {}: {e:#}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }
    if paths.is_empty() {
        println!("nothing to run");
        return ExitCode::FAILURE;
    }

    // The header goes out before the records, because regenerating is a whole-file overwrite
    // (`--write-expectations > the-file`) and anything the file explained about itself is gone
    // the first time a contributor follows the documented workflow. Emitting it here means the
    // format stays documented no matter how often the baseline moves.
    if args.write_expectations {
        print!("{EXPECTATIONS_HEADER}");
    }

    let mut all_ok = true;
    let mut rows = Vec::with_capacity(paths.len());
    for path in &paths {
        let (ok, row) = run_or_expect_error(&args.wpt_root, path, &expect, mode);
        all_ok &= ok;
        rows.push(row);
    }

    // Not while regenerating: `--write-expectations` writes the new baseline to stdout, and a
    // summary in the middle of it would land in the committed file.
    match mode {
        Reporting::Record => {}
        Reporting::Quiet => print_shortlist(&rows),
        _ => print_summary(&rows),
    }

    if let Some(report) = args.report.as_deref() {
        if let Err(e) = write_report(report, &rows, &args.wpt_root) {
            println!("could not write the report: {e:#}");
            return ExitCode::FAILURE;
        }
    }

    // `--shortlist` asks a question rather than asserting anything, so it succeeds as long as it
    // could answer. Inheriting the run's exit code would make it fail whenever the engine has
    // any failing subtest at all - which is the entire reason someone runs it.
    if all_ok || mode == Reporting::Quiet {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
