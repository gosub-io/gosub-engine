//! Minimal WPT runner: executes `.any.js` test files in a bare V8 context with
//! the gosub_jsapi bindings installed, using wpt's own testharness.js.
//!
//! Usage: wpt-run <wpt-root> <test.any.js>...
//!
//! The wpt root is a checkout of <https://github.com/web-platform-tests/wpt>
//! (a sparse checkout of `resources/` plus the test directories is enough).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;

use gosub_jsapi::base64;
use gosub_jsapi::dom_exception::DomException;
use gosub_jsapi::text_encoding::{TextDecoder, TextEncoder};
use gosub_shared::types::Result;
use gosub_v8::{V8Context, V8Engine, V8Function, V8Object};
use gosub_webexecutor::js::{
    Args, IntoWebValue, WebContext, WebFunction, WebFunctionCallBack, WebObject, WebRuntime, WebValue,
};

const PRELUDE: &str = include_str!("prelude.js");

/// Registered after testharness.js so the harness reports into a global the
/// driver can read back out.
const RESULTS_HOOK: &str = r#"
// The driver runs scripts one at a time and V8 flushes microtasks between
// them, so ShellTestEnvironment's "all loaded" microtask fires before the test
// file has even run. Explicit done() from the driver replaces it.
setup({ explicit_done: true });

globalThis.__wpt_results = null;
add_completion_callback(function (tests, harness_status) {
    globalThis.__wpt_results = {
        harness_status: {
            status: harness_status.status,
            message: harness_status.message == null ? null : String(harness_status.message),
        },
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

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 3 {
        eprintln!("Usage: wpt-run <wpt-root> <test.any.js>...");
        return ExitCode::from(2);
    }

    let wpt_root = PathBuf::from(&argv[1]);
    if !wpt_root.join("resources/testharness.js").is_file() {
        eprintln!(
            "error: {} does not look like a wpt checkout (resources/testharness.js missing)",
            wpt_root.display()
        );
        return ExitCode::from(2);
    }

    let mut runtime = V8Engine::new();
    let mut pass = 0usize;
    let mut fail = 0usize;

    for test in &argv[2..] {
        match run_test_file(&mut runtime, &wpt_root, Path::new(test)) {
            Ok((p, f)) => {
                pass += p;
                fail += f;
            }
            Err(e) => {
                eprintln!("error running {test}: {e}");
                fail += 1;
            }
        }
    }

    println!();
    println!("total: {pass} passed, {fail} failed");
    if fail > 0 {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run_test_file(runtime: &mut V8Engine, wpt_root: &Path, test_path: &Path) -> Result<(usize, usize)> {
    println!("=== {} ===", test_path.display());

    let test_src = std::fs::read_to_string(test_path)?;
    let test_dir = test_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let file_name = test_path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Fresh context per test file, like each wpt test gets a fresh global
    let mut ctx: V8Context = runtime.new_context()?;
    register_natives(&mut ctx, wpt_root.to_path_buf(), test_dir.clone())?;

    ctx.run(&format!(
        "globalThis.__gosub_test_name = {};",
        serde_json::to_string(&file_name)?
    ))?;
    ctx.run(PRELUDE)?;

    let harness = std::fs::read_to_string(wpt_root.join("resources/testharness.js"))?;
    ctx.run(&harness)?;
    ctx.run(RESULTS_HOOK)?;

    for script in meta_scripts(&test_src) {
        let path = match script.strip_prefix('/') {
            Some(absolute) => wpt_root.join(absolute),
            None => test_dir.join(&script),
        };
        let src = std::fs::read_to_string(&path)?;
        ctx.run(&src)?;
    }

    if let Err(e) = ctx.run(&test_src) {
        println!("uncaught error while running test file: {e}");
    }
    ctx.run("done();")?;

    // Let the harness settle: every run() call flushes microtasks, and the
    // drain runs whatever the shims queued as timers.
    let mut results_json = None;
    for _ in 0..100 {
        if ctx.run("globalThis.__wpt_results !== null")?.as_string()? == "true" {
            results_json = Some(ctx.run("JSON.stringify(globalThis.__wpt_results)")?.as_string()?);
            break;
        }
        ctx.run("__drainTimers();")?;
    }

    let Some(results_json) = results_json else {
        println!("FAIL: harness never completed");
        return Ok((0, 1));
    };

    report(&results_json)
}

/// Install `__gosub__.{atob,btoa,readRelative}` on the global object. The
/// prelude moves them to their proper places and deletes `__gosub__`.
fn register_natives(ctx: &mut V8Context, wpt_root: PathBuf, test_dir: PathBuf) -> Result<()> {
    let obj = V8Object::new(ctx.clone())?;

    let atob = jsapi_string_fn(ctx, base64::atob)?;
    obj.set_method("atob", &atob)?;

    let btoa = jsapi_string_fn(ctx, base64::btoa)?;
    obj.set_method("btoa", &btoa)?;

    let read_relative = V8Function::new(ctx.clone(), move |cb| {
        let ctx = cb.context();
        let Some(arg) = cb.args().get(0, ctx.clone()) else {
            cb.error("readRelative requires a path argument");
            return;
        };
        let path = match arg.as_string() {
            Ok(p) => p,
            Err(e) => {
                cb.error(e);
                return;
            }
        };

        let resolved = match path.strip_prefix('/') {
            Some(absolute) => wpt_root.join(absolute),
            None => test_dir.join(&path),
        };

        match std::fs::read_to_string(&resolved) {
            Ok(text) => match text.to_web_value(ctx) {
                Ok(v) => cb.ret(v),
                Err(e) => cb.error(e),
            },
            Err(e) => cb.error(format!("failed to read {}: {e}", resolved.display())),
        }
    })?;
    obj.set_method("readRelative", &read_relative)?;

    let te_encode = jsapi_string_fn(ctx, |input| Ok(bytes_to_binary_string(&TextEncoder::new().encode(input))))?;
    obj.set_method("teEncode", &te_encode)?;

    // TextDecoders are stateful (streaming); JS holds an id into this map
    let decoders: Rc<RefCell<HashMap<u32, TextDecoder>>> = Rc::new(RefCell::new(HashMap::new()));
    let next_id = Rc::new(RefCell::new(0u32));

    let td_new = {
        let decoders = Rc::clone(&decoders);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let (Some(label), Some(fatal), Some(ignore_bom)) = (
                arg_string(cb, 0),
                arg_number(cb, 1),
                arg_number(cb, 2),
            ) else {
                cb.error("tdNew requires (label, fatal, ignoreBOM) arguments");
                return;
            };

            match TextDecoder::new(&label, fatal != 0.0, ignore_bom != 0.0) {
                Ok(decoder) => {
                    let mut id_ref = next_id.borrow_mut();
                    *id_ref += 1;
                    let id = *id_ref;
                    decoders.borrow_mut().insert(id, decoder);
                    match f64::from(id).to_web_value(ctx) {
                        Ok(v) => cb.ret(v),
                        Err(e) => cb.error(e),
                    }
                }
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("tdNew", &td_new)?;

    let td_encoding = {
        let decoders = Rc::clone(&decoders);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let Some(id) = arg_number(cb, 0) else {
                cb.error("tdEncoding requires a decoder id");
                return;
            };
            let name = decoders.borrow().get(&(id as u32)).map(TextDecoder::encoding);
            match name {
                Some(name) => match name.to_web_value(ctx) {
                    Ok(v) => cb.ret(v),
                    Err(e) => cb.error(e),
                },
                None => cb.error("tdEncoding: unknown decoder id"),
            }
        })?
    };
    obj.set_method("tdEncoding", &td_encoding)?;

    let td_decode = {
        let decoders = Rc::clone(&decoders);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let (Some(id), Some(input), Some(stream)) = (
                arg_number(cb, 0),
                arg_string(cb, 1),
                arg_number(cb, 2),
            ) else {
                cb.error("tdDecode requires (id, bytes, stream) arguments");
                return;
            };
            let Some(bytes) = binary_string_to_bytes(&input) else {
                cb.error("tdDecode: input is not a binary string");
                return;
            };

            let mut decoders = decoders.borrow_mut();
            let Some(decoder) = decoders.get_mut(&(id as u32)) else {
                cb.error("tdDecode: unknown decoder id");
                return;
            };

            match decoder.decode(&bytes, stream != 0.0) {
                Ok(out) => match out.to_web_value(ctx) {
                    Ok(v) => cb.ret(v),
                    Err(e) => cb.error(e),
                },
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("tdDecode", &td_decode)?;

    ctx.set_on_global_object("__gosub__", obj.into())?;

    Ok(())
}

/// Bytes as a JS "binary string": one code point in U+0000..=U+00FF per byte.
/// The prelude converts these to/from Uint8Array at the JS boundary.
fn bytes_to_binary_string(bytes: &[u8]) -> String {
    bytes.iter().copied().map(char::from).collect()
}

fn binary_string_to_bytes(s: &str) -> Option<Vec<u8>> {
    s.chars().map(|c| u8::try_from(c as u32).ok()).collect()
}

fn arg_string(cb: &mut gosub_v8::V8FunctionCallBack, index: usize) -> Option<String> {
    let ctx = cb.context();
    cb.args().get(index, ctx).and_then(|v| v.as_string().ok())
}

fn arg_number(cb: &mut gosub_v8::V8FunctionCallBack, index: usize) -> Option<f64> {
    let ctx = cb.context();
    cb.args().get(index, ctx).and_then(|v| v.as_number().ok())
}

/// Wrap a string-in/string-out jsapi function as a JS function. A DomException
/// error is thrown as a JS Error whose message carries "Name: message"; the
/// prelude rethrows those as proper DOMExceptions.
fn jsapi_string_fn(
    ctx: &V8Context,
    f: impl Fn(&str) -> std::result::Result<String, DomException> + 'static,
) -> Result<V8Function> {
    V8Function::new(ctx.clone(), move |cb| {
        let ctx = cb.context();
        let Some(arg) = cb.args().get(0, ctx.clone()) else {
            cb.error("missing required argument");
            return;
        };
        let input = match arg.as_string() {
            Ok(s) => s,
            Err(e) => {
                cb.error(e);
                return;
            }
        };

        match f(&input) {
            Ok(out) => match out.to_web_value(ctx) {
                Ok(v) => cb.ret(v),
                Err(e) => cb.error(e),
            },
            Err(e) => cb.error(e),
        }
    })
}

/// Header `// META: script=...` directives of a wpt test file
fn meta_scripts(src: &str) -> Vec<String> {
    let mut scripts = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if !line.is_empty() && !line.starts_with("//") {
            break;
        }
        if let Some(meta) = line.strip_prefix("// META:") {
            if let Some(script) = meta.trim().strip_prefix("script=") {
                scripts.push(script.trim().to_owned());
            }
        }
    }
    scripts
}

fn report(results_json: &str) -> Result<(usize, usize)> {
    let results: serde_json::Value = serde_json::from_str(results_json)?;

    let mut pass = 0usize;
    let mut fail = 0usize;

    for test in results["tests"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let name = test["name"].as_str().unwrap_or("<unnamed>");
        let status = test["status"].as_u64().unwrap_or(u64::MAX);
        if status == 0 {
            pass += 1;
            if std::env::var_os("WPT_RUN_VERBOSE").is_some() {
                println!("PASS: {name}");
            }
            continue;
        }

        fail += 1;
        let status = match status {
            1 => "FAIL",
            2 => "TIMEOUT",
            3 => "NOTRUN",
            4 => "PRECONDITION_FAILED",
            _ => "UNKNOWN",
        };
        let message = test["message"].as_str().unwrap_or("");
        println!("{status}: {name} — {message}");
    }

    let harness_status = results["harness_status"]["status"].as_u64().unwrap_or(0);
    if harness_status != 0 {
        fail += 1;
        let message = results["harness_status"]["message"].as_str().unwrap_or("");
        println!("HARNESS ERROR (status {harness_status}): {message}");
    }

    println!("{pass} passed, {fail} failed");
    Ok((pass, fail))
}
