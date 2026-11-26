//! The `CSS` global: `CSS.supports()` and `CSS.escape()`.
//!
//! `CSS.supports` asks the question the engine already answers for `@supports`, so this is a
//! binding rather than an implementation - [`gosub_css3::supports::supports_declaration`] is
//! the same gate an `@supports` block goes through, and its doc comment has said "and
//! (eventually) `CSS.supports()`" since it was written.
//!
//! It is worth more than it looks. wpt's `test_computed_value` opens every one of its subtests
//! with `assert_true(CSS.supports(property, specified))`, so without this global the *whole*
//! computed-value corpus dies on its first line - 15 suites and 563 subtests in `css-values`
//! alone, none of which were reaching the engine at all.

use gosub_css3::supports::{supports_declaration, SupportsCondition};
use rquickjs::{Ctx, Result};

/// `CSS.supports()`, in both of the forms the CSSOM defines.
///
/// One argument is a condition as it would be written after `@supports`
/// (`CSS.supports("(display: grid)")`); two are a property and a value
/// (`CSS.supports("display", "grid")`). The two-argument form is *not* a condition - a value
/// containing `)` must not be able to close a parenthesis that was never opened - so it goes
/// straight to the declaration check rather than being spliced into a string.
#[rquickjs::function]
fn supports(first: String, second: Option<String>) -> bool {
    match second {
        Some(value) => supports_declaration(&first, &value),
        // A bare condition still has to be parenthesised, as in the at-rule. `@supports`
        // prelude syntax is what this argument is defined to be.
        None => SupportsCondition::parse(&first).matches(),
    }
}

/// `CSS.escape()`: a string as a CSS identifier, per cssom-1.
///
/// Present because it is cheap and because wpt's helpers reach for it when building selectors,
/// where an unescaped id starting with a digit would otherwise produce an invalid selector
/// rather than a failing assertion.
#[rquickjs::function]
fn escape(value: String) -> String {
    let mut out = String::with_capacity(value.len());
    for (i, c) in value.chars().enumerate() {
        match c {
            // NULL becomes the replacement character rather than being dropped.
            '\0' => out.push('\u{FFFD}'),
            // A leading digit, or a digit after a leading `-`, has to be escaped as a code
            // point or it would read as the start of a number.
            '0'..='9' if i == 0 || (i == 1 && value.starts_with('-')) => {
                out.push_str(&format!("\\{:x} ", c as u32));
            }
            // A lone `-` is not an identifier on its own.
            '-' if i == 0 && value.len() == 1 => {
                out.push('\\');
                out.push('-');
            }
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii() => out.push(c),
            // The C0 controls and DEL have no printable escape.
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\{:x} ", c as u32));
            }
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
    out
}

pub(crate) fn install(ctx: &Ctx<'_>) -> Result<()> {
    let namespace = rquickjs::Object::new(ctx.clone())?;
    namespace.set("supports", js_supports)?;
    namespace.set("escape", js_escape)?;
    ctx.globals().set("CSS", namespace)?;
    Ok(())
}
