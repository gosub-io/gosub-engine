//! Minimal WPT runner: executes `.any.js` test files in a bare V8 context with
//! the gosub_jsapi bindings installed, using wpt's own testharness.js.
//!
//! Usage: wpt-run <wpt-root> <test.any.js>...
//!
//! The wpt root is a checkout of <https://github.com/web-platform-tests/wpt>
//! (a sparse checkout of `resources/` plus the test directories is enough).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;

use gosub_jsapi::base64;
use gosub_jsapi::console::{Console, LogLevel, Printer};
use gosub_jsapi::dom_exception::DomException;
use gosub_jsapi::text_encoding::{TextDecoder, TextEncoder};
use gosub_jsapi::url::{Url, UrlSearchParams};
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
    let mut idx = 1;

    // --expect FILE: test names listed there (one per line, # comments) are
    // known upstream failures — reported as XFAIL, and flagged if they pass.
    let mut expected: HashSet<String> = HashSet::new();
    if argv.get(idx).is_some_and(|a| a == "--expect") {
        let Some(path) = argv.get(idx + 1) else {
            eprintln!("--expect requires a file argument");
            return ExitCode::from(2);
        };
        match std::fs::read_to_string(path) {
            Ok(s) => {
                // Only strip \r — test names can legitimately end with spaces
                expected = s
                    .lines()
                    .map(|l| l.trim_end_matches('\r'))
                    .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                    .map(str::to_owned)
                    .collect();
            }
            Err(e) => {
                eprintln!("cannot read expectations file {path}: {e}");
                return ExitCode::from(2);
            }
        }
        idx += 2;
    }

    if argv.len() < idx + 2 {
        eprintln!("Usage: wpt-run [--expect FILE] <wpt-root> <test.any.js>...");
        return ExitCode::from(2);
    }

    let wpt_root = PathBuf::from(&argv[idx]);
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
    let mut xfail = 0usize;

    for test in &argv[idx + 1..] {
        match run_test_file(&mut runtime, &wpt_root, Path::new(test), &expected) {
            Ok((p, f, x)) => {
                pass += p;
                fail += f;
                xfail += x;
            }
            Err(e) => {
                eprintln!("error running {test}: {e}");
                fail += 1;
            }
        }
    }

    println!();
    println!("total: {pass} passed, {fail} failed, {xfail} known failures");
    if fail > 0 {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run_test_file(
    runtime: &mut V8Engine,
    wpt_root: &Path,
    test_path: &Path,
    expected: &HashSet<String>,
) -> Result<(usize, usize, usize)> {
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
        return Ok((0, 1, 0));
    };

    report(&results_json, expected)
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

    register_console_native(ctx, &obj)?;
    register_url_natives(ctx, &obj)?;

    ctx.set_on_global_object("__gosub__", obj.into())?;

    Ok(())
}

/// Prints console output straight to the runner's stdout
struct StdoutPrinter;

impl Printer for StdoutPrinter {
    fn print(&mut self, log_level: LogLevel, args: &[&dyn std::fmt::Display], _options: &[&str]) {
        let joined = args.iter().map(ToString::to_string).collect::<Vec<_>>().join(" ");
        println!("[console.{log_level}] {joined}");
    }

    fn clear(&mut self) {}

    fn end_group(&mut self) {}
}

/// One `consoleCall(method, argsJson)` native dispatching onto a per-context
/// jsapi Console. Args arrive pre-stringified from the prelude (JSON array;
/// assert's first element is the boolean condition).
fn register_console_native(ctx: &mut V8Context, obj: &V8Object) -> Result<()> {
    let console = Rc::new(RefCell::new(Console::new(Box::new(StdoutPrinter))));

    let console_call = V8Function::new(ctx.clone(), move |cb| {
        let (Some(method), Some(args_json)) = (arg_string(cb, 0), arg_string(cb, 1)) else {
            cb.error("consoleCall requires (method, argsJson) arguments");
            return;
        };

        let parsed: Vec<serde_json::Value> = match serde_json::from_str(&args_json) {
            Ok(v) => v,
            Err(e) => {
                cb.error(format!("consoleCall: bad args JSON: {e}"));
                return;
            }
        };
        let strings: Vec<String> = parsed
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_owned())
            .collect();
        let refs: Vec<&dyn std::fmt::Display> = strings.iter().map(|s| s as &dyn std::fmt::Display).collect();
        let first = strings.first().map(String::as_str).unwrap_or_default();

        let mut console = console.borrow_mut();
        match method.as_str() {
            "assert" => {
                let condition = parsed.first().and_then(serde_json::Value::as_bool).unwrap_or(false);
                console.assert(condition, refs.get(1..).unwrap_or(&[]));
            }
            "log" => console.log(&refs),
            "debug" => console.debug(&refs),
            "info" => console.info(&refs),
            "warn" => console.warn(&refs),
            "error" => console.error(&refs),
            "trace" => console.trace(&refs),
            "dirxml" => console.dirxml(&refs),
            "dir" => console.dir(&first, &[]),
            "table" => console.table(first.to_owned(), &[]),
            "group" => console.group(&refs),
            "groupCollapsed" => console.group_collapsed(&refs),
            "groupEnd" => {
                console.group_end();
            }
            "clear" => console.clear(),
            "count" => console.count(first),
            "countReset" => console.count_reset(first),
            "time" => console.time(first),
            "timeLog" => console.time_log(first, refs.get(1..).unwrap_or(&[])),
            "timeEnd" => console.time_end(first),
            _ => {
                cb.error(format!("consoleCall: unknown method '{method}'"));
                return;
            }
        }
        drop(console);
        ret_undefined(cb);
    })?;
    obj.set_method("consoleCall", &console_call)?;

    Ok(())
}

/// URL objects and URLSearchParams lists, with the spec's mutual linkage: a
/// params list created via `url.searchParams` writes back into the URL's query
/// on every mutation, and the URL's `href`/`search` setters reinitialize the
/// linked list.
#[derive(Default)]
struct UrlStore {
    urls: HashMap<u32, (Url, Option<u32>)>,
    params: HashMap<u32, (UrlSearchParams, Option<u32>)>,
    next: u32,
}

impl UrlStore {
    fn alloc(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    fn sync_params_from_url(&mut self, url_id: u32) {
        let (query, params_id) = match self.urls.get(&url_id) {
            Some((url, Some(params_id))) => (url.query().unwrap_or("").to_owned(), *params_id),
            _ => return,
        };
        if let Some((list, _)) = self.params.get_mut(&params_id) {
            list.reset_from_query(&query);
        }
    }

    fn sync_url_from_params(&mut self, params_id: u32) {
        let (serialized, url_id) = match self.params.get(&params_id) {
            Some((list, Some(url_id))) => (list.to_query_string(), *url_id),
            _ => return,
        };
        if let Some((url, _)) = self.urls.get_mut(&url_id) {
            if serialized.is_empty() {
                url.clear_query_for_params();
            } else {
                url.set_search(&serialized);
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
fn register_url_natives(ctx: &mut V8Context, obj: &V8Object) -> Result<()> {
    let store: Rc<RefCell<UrlStore>> = Rc::new(RefCell::new(UrlStore::default()));

    let url_new = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let (Some(input), Some(has_base), Some(base)) =
                (arg_string(cb, 0), arg_number(cb, 1), arg_string(cb, 2))
            else {
                cb.error("urlNew requires (input, hasBase, base) arguments");
                return;
            };

            let base = (has_base != 0.0).then_some(base.as_str());
            match Url::parse(&input, base) {
                Ok(url) => {
                    let mut store = store.borrow_mut();
                    let id = store.alloc();
                    store.urls.insert(id, (url, None));
                    match f64::from(id).to_web_value(ctx) {
                        Ok(v) => cb.ret(v),
                        Err(e) => cb.error(e),
                    }
                }
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("urlNew", &url_new)?;

    let url_get = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let (Some(id), Some(prop)) = (arg_number(cb, 0), arg_string(cb, 1)) else {
                cb.error("urlGet requires (id, prop) arguments");
                return;
            };

            let store = store.borrow();
            let Some((url, _)) = store.urls.get(&(id as u32)) else {
                cb.error("urlGet: unknown url id");
                return;
            };

            let value = match prop.as_str() {
                "href" => url.href().to_owned(),
                "origin" => url.origin(),
                "protocol" => url.protocol().to_owned(),
                "username" => url.username().to_owned(),
                "password" => url.password().to_owned(),
                "host" => url.host().to_owned(),
                "hostname" => url.hostname().to_owned(),
                "port" => url.port().to_owned(),
                "pathname" => url.pathname().to_owned(),
                "search" => url.search().to_owned(),
                "hash" => url.hash().to_owned(),
                _ => {
                    cb.error(format!("urlGet: unknown property '{prop}'"));
                    return;
                }
            };
            match value.to_web_value(ctx) {
                Ok(v) => cb.ret(v),
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("urlGet", &url_get)?;

    let url_set = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let (Some(id), Some(prop), Some(value)) = (arg_number(cb, 0), arg_string(cb, 1), arg_string(cb, 2))
            else {
                cb.error("urlSet requires (id, prop, value) arguments");
                return;
            };
            let id = id as u32;

            let mut store = store.borrow_mut();
            let Some((url, _)) = store.urls.get_mut(&id) else {
                cb.error("urlSet: unknown url id");
                return;
            };

            match prop.as_str() {
                "href" => {
                    if let Err(e) = url.set_href(&value) {
                        cb.error(e);
                        return;
                    }
                    store.sync_params_from_url(id);
                }
                "search" => {
                    url.set_search(&value);
                    store.sync_params_from_url(id);
                }
                "protocol" => url.set_protocol(&value),
                "username" => url.set_username(&value),
                "password" => url.set_password(&value),
                "host" => url.set_host(&value),
                "hostname" => url.set_hostname(&value),
                "port" => url.set_port(&value),
                "pathname" => url.set_pathname(&value),
                "hash" => url.set_hash(&value),
                _ => {
                    cb.error(format!("urlSet: unknown property '{prop}'"));
                    return;
                }
            }
            ret_undefined(cb);
        })?
    };
    obj.set_method("urlSet", &url_set)?;

    let url_search_params_id = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let Some(id) = arg_number(cb, 0) else {
                cb.error("urlSearchParamsId requires a url id");
                return;
            };
            let id = id as u32;

            let mut store = store.borrow_mut();
            let Some((url, linked)) = store.urls.get(&(id)) else {
                cb.error("urlSearchParamsId: unknown url id");
                return;
            };

            let params_id = if let Some(params_id) = linked {
                *params_id
            } else {
                let list = UrlSearchParams::parse_query(url.query().unwrap_or(""));
                let params_id = store.alloc();
                store.params.insert(params_id, (list, Some(id)));
                if let Some((_, linked)) = store.urls.get_mut(&id) {
                    *linked = Some(params_id);
                }
                params_id
            };

            match f64::from(params_id).to_web_value(ctx) {
                Ok(v) => cb.ret(v),
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("urlSearchParamsId", &url_search_params_id)?;

    let sp_new = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let Some(init) = arg_string(cb, 0) else {
                cb.error("spNew requires an init string");
                return;
            };

            let mut store = store.borrow_mut();
            let id = store.alloc();
            store.params.insert(id, (UrlSearchParams::parse_query(&init), None));
            match f64::from(id).to_web_value(ctx) {
                Ok(v) => cb.ret(v),
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("spNew", &sp_new)?;

    let sp_append = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let (Some(id), Some(name), Some(value)) = (arg_number(cb, 0), arg_string(cb, 1), arg_string(cb, 2))
            else {
                cb.error("spAppend requires (id, name, value) arguments");
                return;
            };
            let id = id as u32;

            let mut store = store.borrow_mut();
            let Some((list, _)) = store.params.get_mut(&id) else {
                cb.error("spAppend: unknown params id");
                return;
            };
            list.append(&name, &value);
            store.sync_url_from_params(id);
            ret_undefined(cb);
        })?
    };
    obj.set_method("spAppend", &sp_append)?;

    let sp_set = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let (Some(id), Some(name), Some(value)) = (arg_number(cb, 0), arg_string(cb, 1), arg_string(cb, 2))
            else {
                cb.error("spSet requires (id, name, value) arguments");
                return;
            };
            let id = id as u32;

            let mut store = store.borrow_mut();
            let Some((list, _)) = store.params.get_mut(&id) else {
                cb.error("spSet: unknown params id");
                return;
            };
            list.set(&name, &value);
            store.sync_url_from_params(id);
            ret_undefined(cb);
        })?
    };
    obj.set_method("spSet", &sp_set)?;

    let sp_delete = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let (Some(id), Some(name), Some(has_value), Some(value)) = (
                arg_number(cb, 0),
                arg_string(cb, 1),
                arg_number(cb, 2),
                arg_string(cb, 3),
            ) else {
                cb.error("spDelete requires (id, name, hasValue, value) arguments");
                return;
            };
            let id = id as u32;

            let mut store = store.borrow_mut();
            let Some((list, _)) = store.params.get_mut(&id) else {
                cb.error("spDelete: unknown params id");
                return;
            };
            list.delete(&name, (has_value != 0.0).then_some(value.as_str()));
            store.sync_url_from_params(id);
            ret_undefined(cb);
        })?
    };
    obj.set_method("spDelete", &sp_delete)?;

    let sp_sort = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let Some(id) = arg_number(cb, 0) else {
                cb.error("spSort requires a params id");
                return;
            };
            let id = id as u32;

            let mut store = store.borrow_mut();
            let Some((list, _)) = store.params.get_mut(&id) else {
                cb.error("spSort: unknown params id");
                return;
            };
            list.sort();
            store.sync_url_from_params(id);
            ret_undefined(cb);
        })?
    };
    obj.set_method("spSort", &sp_sort)?;

    let sp_get = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let (Some(id), Some(name)) = (arg_number(cb, 0), arg_string(cb, 1)) else {
                cb.error("spGet requires (id, name) arguments");
                return;
            };

            let store = store.borrow();
            let Some((list, _)) = store.params.get(&(id as u32)) else {
                cb.error("spGet: unknown params id");
                return;
            };
            match serde_json::to_string(&list.get(&name)) {
                Ok(json) => match json.to_web_value(ctx) {
                    Ok(v) => cb.ret(v),
                    Err(e) => cb.error(e),
                },
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("spGet", &sp_get)?;

    let sp_get_all = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let (Some(id), Some(name)) = (arg_number(cb, 0), arg_string(cb, 1)) else {
                cb.error("spGetAll requires (id, name) arguments");
                return;
            };

            let store = store.borrow();
            let Some((list, _)) = store.params.get(&(id as u32)) else {
                cb.error("spGetAll: unknown params id");
                return;
            };
            match serde_json::to_string(&list.get_all(&name)) {
                Ok(json) => match json.to_web_value(ctx) {
                    Ok(v) => cb.ret(v),
                    Err(e) => cb.error(e),
                },
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("spGetAll", &sp_get_all)?;

    let sp_has = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let (Some(id), Some(name), Some(has_value), Some(value)) = (
                arg_number(cb, 0),
                arg_string(cb, 1),
                arg_number(cb, 2),
                arg_string(cb, 3),
            ) else {
                cb.error("spHas requires (id, name, hasValue, value) arguments");
                return;
            };

            let store = store.borrow();
            let Some((list, _)) = store.params.get(&(id as u32)) else {
                cb.error("spHas: unknown params id");
                return;
            };
            let found = list.has(&name, (has_value != 0.0).then_some(value.as_str()));
            match f64::from(u8::from(found)).to_web_value(ctx) {
                Ok(v) => cb.ret(v),
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("spHas", &sp_has)?;

    let sp_size = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let Some(id) = arg_number(cb, 0) else {
                cb.error("spSize requires a params id");
                return;
            };

            let store = store.borrow();
            let Some((list, _)) = store.params.get(&(id as u32)) else {
                cb.error("spSize: unknown params id");
                return;
            };
            #[allow(clippy::cast_precision_loss)]
            match (list.len() as f64).to_web_value(ctx) {
                Ok(v) => cb.ret(v),
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("spSize", &sp_size)?;

    let sp_entry_at = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let (Some(id), Some(index)) = (arg_number(cb, 0), arg_number(cb, 1)) else {
                cb.error("spEntryAt requires (id, index) arguments");
                return;
            };

            let store = store.borrow();
            let Some((list, _)) = store.params.get(&(id as u32)) else {
                cb.error("spEntryAt: unknown params id");
                return;
            };
            let entry = list.entries().get(index as usize);
            match serde_json::to_string(&entry) {
                Ok(json) => match json.to_web_value(ctx) {
                    Ok(v) => cb.ret(v),
                    Err(e) => cb.error(e),
                },
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("spEntryAt", &sp_entry_at)?;

    let sp_to_string = {
        let store = Rc::clone(&store);
        V8Function::new(ctx.clone(), move |cb| {
            let ctx = cb.context();
            let Some(id) = arg_number(cb, 0) else {
                cb.error("spToString requires a params id");
                return;
            };

            let store = store.borrow();
            let Some((list, _)) = store.params.get(&(id as u32)) else {
                cb.error("spToString: unknown params id");
                return;
            };
            match list.to_query_string().to_web_value(ctx) {
                Ok(v) => cb.ret(v),
                Err(e) => cb.error(e),
            }
        })?
    };
    obj.set_method("spToString", &sp_to_string)?;

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

/// Explicit undefined return for void natives — a callback that never calls
/// `ret()` is treated as an error by the function glue.
fn ret_undefined(cb: &mut gosub_v8::V8FunctionCallBack) {
    let ctx = cb.context();
    match gosub_v8::V8Value::new_undefined(ctx) {
        Ok(v) => cb.ret(v),
        Err(e) => cb.error(e),
    }
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

/// Test names go into a line-based expectations file, so embedded control
/// characters (some URL tests put raw newlines/tabs in names) are escaped.
fn normalize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

fn report(results_json: &str, expected: &HashSet<String>) -> Result<(usize, usize, usize)> {
    let results: serde_json::Value = serde_json::from_str(results_json)?;

    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut xfail = 0usize;

    for test in results["tests"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let name = normalize_name(test["name"].as_str().unwrap_or("<unnamed>"));
        let name = name.as_str();
        let status = test["status"].as_u64().unwrap_or(u64::MAX);
        if status == 0 {
            if expected.contains(name) {
                fail += 1;
                println!("UNEXPECTED PASS: {name} — remove it from the expectations file");
            } else {
                pass += 1;
                if std::env::var_os("WPT_RUN_VERBOSE").is_some() {
                    println!("PASS: {name}");
                }
            }
            continue;
        }

        if expected.contains(name) {
            xfail += 1;
            if std::env::var_os("WPT_RUN_VERBOSE").is_some() {
                println!("XFAIL: {name}");
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

    println!("{pass} passed, {fail} failed, {xfail} known failures");
    Ok((pass, fail, xfail))
}
