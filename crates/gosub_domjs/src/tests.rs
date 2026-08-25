//! Smoke tests: the bindings must read the real document, not a JS-side copy.

use rquickjs::{CatchResultExt, Context, Runtime};

use crate::timers::{TimerState, Timers};
use crate::{install, parse_document};

/// Run `script`, drain the timer queue, then read `after` back out - the timer tests need to
/// observe state that only exists once the queue has been pumped.
fn eval_after_timers(html: &str, script: &str, after: &str) -> String {
    let (doc, _) = parse_document(html, None).expect("parse");
    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let timers: Timers = std::rc::Rc::new(std::cell::RefCell::new(TimerState::default()));
    context.with(|ctx| {
        install(&ctx, doc, &timers).expect("install");
        ctx.eval::<(), _>(script).catch(&ctx).expect("eval");
        while crate::timers::run_next(&ctx, &timers).expect("timers") {}
        ctx.eval::<String, _>(after).catch(&ctx).expect("read back")
    })
}

fn eval(html: &str, script: &str) -> String {
    let (doc, _) = parse_document(html, None).expect("parse");
    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let timers: Timers = std::rc::Rc::new(std::cell::RefCell::new(TimerState::default()));
    context.with(|ctx| {
        install(&ctx, doc, &timers).expect("install");
        let result = ctx.eval::<String, _>(script).catch(&ctx).expect("eval");
        while crate::timers::run_next(&ctx, &timers).expect("timers") {}
        result
    })
}

#[test]
fn reads_parsed_markup() {
    let value = eval(
        "<select><option value=a>First</option><option>Second</option></select>",
        "document.getElementsByTagName('option')[1].value",
    );
    assert_eq!(value, "Second");
}

#[test]
fn option_value_falls_back_to_stripped_and_collapsed_text() {
    let value = eval(
        "<option> child  node </option>",
        "document.querySelector('option').value",
    );
    assert_eq!(value, "child node");
}

#[test]
fn mutations_land_in_the_document() {
    let html = eval(
        "<body><p id=target></p></body>",
        "const p = document.getElementById('target');
         p.appendChild(document.createTextNode('hi'));
         p.setAttribute('data-x', '1');
         p.outerHTML",
    );
    assert!(html.contains("hi"), "{html}");
    assert!(html.contains("data-x"), "{html}");
}

#[test]
fn node_wrappers_have_stable_identity() {
    let same = eval(
        "<div id=parent><span id=child></span></div>",
        "String(document.getElementById('child').parentNode === document.getElementById('parent'))",
    );
    assert_eq!(same, "true");
}

#[test]
fn listeners_run_in_capture_target_bubble_order() {
    let order = eval(
        "<div id=outer><span id=inner></span></div>",
        "const seen = [];
         const outer = document.getElementById('outer');
         const inner = document.getElementById('inner');
         outer.addEventListener('x', () => seen.push('capture'), true);
         outer.addEventListener('x', () => seen.push('bubble'));
         inner.addEventListener('x', e => seen.push('target' + e.eventPhase));
         inner.dispatchEvent(new Event('x', {bubbles: true}));
         seen.join(',')",
    );
    assert_eq!(order, "capture,target2,bubble");
}

#[test]
fn duplicate_listeners_are_ignored_and_once_runs_once() {
    let count = eval(
        "<span id=el></span>",
        "let n = 0;
         const el = document.getElementById('el');
         const handler = () => n++;
         el.addEventListener('y', handler, {once: true});
         el.addEventListener('y', handler, {once: true});
         el.dispatchEvent(new Event('y'));
         el.dispatchEvent(new Event('y'));
         String(n)",
    );
    assert_eq!(count, "1");
}

#[test]
fn prevent_default_is_reported_by_dispatch_event() {
    let result = eval(
        "<span id=el></span>",
        "const el = document.getElementById('el');
         el.addEventListener('z', e => e.preventDefault());
         String(el.dispatchEvent(new Event('z', {cancelable: true})))",
    );
    assert_eq!(result, "false");
}

#[test]
fn stop_propagation_keeps_an_event_from_the_parent() {
    let seen = eval(
        "<div id=outer><span id=inner></span></div>",
        "const seen = [];
         document.getElementById('outer').addEventListener('s', () => seen.push('outer'));
         const inner = document.getElementById('inner');
         inner.addEventListener('s', e => { seen.push('inner'); e.stopPropagation(); });
         inner.dispatchEvent(new Event('s', {bubbles: true}));
         seen.join(',')",
    );
    assert_eq!(seen, "inner");
}

#[test]
fn timers_fire_in_due_order_not_registration_order() {
    let order = eval_after_timers(
        "<span></span>",
        "globalThis.seen = [];
         setTimeout(() => seen.push('late'), 50);
         setTimeout(() => seen.push('early'), 1);
         setTimeout(() => seen.push('same-time-second'), 1);",
        "seen.join(',')",
    );
    assert_eq!(order, "early,same-time-second,late");
}

#[test]
fn a_cleared_timer_never_runs() {
    let ran = eval_after_timers(
        "<span></span>",
        "globalThis.ran = false;
         const id = setTimeout(() => { ran = true; }, 5);
         clearTimeout(id);",
        "String(ran)",
    );
    assert_eq!(ran, "false");
}

#[test]
fn request_animation_frame_delivers_a_timestamp() {
    let kind = eval_after_timers(
        "<span></span>",
        "globalThis.kind = 'never ran';
         requestAnimationFrame(ts => { kind = typeof ts; });",
        "kind",
    );
    assert_eq!(kind, "number");
}

#[test]
fn click_reaches_a_listener_on_an_ancestor() {
    let seen = eval(
        "<form id=f><button id=b>go</button></form>",
        "const seen = [];
         document.getElementById('f').addEventListener('click', e => seen.push(e.target.tagName));
         document.getElementById('b').click();
         seen.join(',')",
    );
    assert_eq!(seen, "BUTTON");
}
