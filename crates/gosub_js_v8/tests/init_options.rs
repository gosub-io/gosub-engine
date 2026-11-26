//! V8 is initialized once per process, so options need a test binary of their own.
#![allow(clippy::unwrap_used)]

use gosub_js_v8::{V8Context, V8ContextOptions, V8Engine, V8Error, V8Options};
use gosub_webexecutor::js::{WebContext, WebValue};

#[test]
fn options_reach_v8_and_the_first_initialization_wins() {
    let options = V8Options {
        jitless: true,
        single_threaded: true,
        flags: vec!["--stack-size=400".to_owned()],
    };
    assert!(V8Engine::initialize_with(&options));
    assert!(!V8Engine::initialize_with(&V8Options::default()));

    let mut ctx = V8Context::with_default().unwrap();

    // jitless V8 has no WebAssembly
    assert_eq!(ctx.run("typeof WebAssembly").unwrap().as_string().unwrap(), "undefined");

    // --stack-size is in KB; the default of about 1 MB gets several times deeper than this
    let depth = ctx
        .run("let n = 0; function f() { n++; f(); } try { f() } catch (e) {} n")
        .unwrap()
        .as_number()
        .unwrap();
    assert!(depth > 100.0 && depth < 20_000.0, "recursion depth {depth}");

    // promises still settle without worker threads
    let settled = ctx
        .run("let v = 0; Promise.resolve(3).then(x => { v = x }); 0")
        .unwrap();
    assert_eq!(settled.as_number().unwrap(), 0.0);
    assert_eq!(ctx.run("v").unwrap().as_number().unwrap(), 3.0);

    heap_limit_ends_an_allocation_loop();
}

// Part of the one test above: it needs the jitless process. With the JIT on, this loop runs
// past the termination request and V8 aborts the process.
fn heap_limit_ends_an_allocation_loop() {
    const HOARD: &str = "(function () { const a = []; for (;;) a.push(new Array(1000).fill(0)); })()";

    let mut ctx = V8Context::with_options(&V8ContextOptions {
        heap_limit: Some(32 * 1024 * 1024),
        ..Default::default()
    })
    .unwrap();

    for _ in 0..3 {
        let error = ctx.run(HOARD).err().unwrap();
        assert_eq!(error.downcast_ref::<V8Error>(), Some(&V8Error::OutOfMemory));
        assert_eq!(ctx.run("1 + 1").unwrap().as_number().unwrap(), 2.0);
    }
}
