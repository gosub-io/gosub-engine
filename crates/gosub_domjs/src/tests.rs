//! Smoke tests: the bindings must read the real document, not a JS-side copy.

use rquickjs::{CatchResultExt, Context, Runtime};

use crate::{install, parse_document};

fn eval(html: &str, script: &str) -> String {
    let (doc, _) = parse_document(html, None).expect("parse");
    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    context.with(|ctx| {
        install(&ctx, doc).expect("install");
        ctx.eval::<String, _>(script).catch(&ctx).expect("eval")
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
