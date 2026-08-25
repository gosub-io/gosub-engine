//! Test-only DOM bindings: a JavaScript view of a parsed gosub document.
//!
//! This exists so the engine can run web-platform-tests that drive form controls from
//! script, long before a real scripting environment lands. It is deliberately *not* a
//! browser: there is no event loop, no navigation, no layout, and scripts run after the
//! document is fully parsed rather than during it.
//!
//! The one rule that keeps this useful: the bindings hold **no DOM logic**. Every property
//! reads or writes the real [`gosub_html5`] document, so a passing test says something about
//! the engine rather than about this crate.

use std::cell::RefCell;
use std::rc::Rc;

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use rquickjs::{Class, Ctx, Object, Value};
use url::Url;

mod document;
pub mod event;
mod node;
mod select;
#[cfg(test)]
mod tests;
mod text;
pub mod timers;

pub use document::GosubDocument;
pub use node::GosubNode;
pub use text::strip_and_collapse;

/// Parse-only module configuration: no renderer, no font system, no layout.
#[derive(Clone, Debug, PartialEq)]
pub struct DomConfig;

impl ModuleConfiguration for DomConfig {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

/// The document type the bindings expose.
pub type Doc = DocumentImpl<DomConfig>;

/// Shared handle to the one document a JS context sees.
pub type DocHandle = Rc<RefCell<Doc>>;

/// Parse `html` into a document. Parse errors are returned rather than raised - a WPT test
/// is allowed to contain markup the parser complains about.
pub fn parse_document(html: &str, url: Option<Url>) -> anyhow::Result<(DocHandle, Vec<String>)> {
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let mut doc = DocumentBuilderImpl::new_document::<DomConfig>(url);
    let errors = Html5Parser::<DomConfig>::parse_document(&mut stream, &mut doc, None)
        .map_err(|e| anyhow::anyhow!("html parse failed: {e}"))?;
    let messages = errors.into_iter().map(|e| e.message).collect();
    Ok((Rc::new(RefCell::new(doc)), messages))
}

/// The JS `Map` that keeps one wrapper object per node, so `a.parentNode === b` holds.
/// It lives on the globals (rather than in Rust) so the wrappers stay reachable for the GC.
const WRAPPER_CACHE: &str = "__gosub_node_wrappers";

/// Install `document` (plus the `window`/`self` aliases) on the globals.
///
/// Call this *after* testharness.js has been evaluated: testharness picks its environment
/// by looking for `document` on the global scope, and the window environment expects a
/// message-passing browser we do not have. Installing afterwards leaves it in shell mode.
pub fn install(ctx: &Ctx<'_>, doc: DocHandle, timers: &timers::Timers) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    event::install(ctx)?;
    timers::install(ctx, timers)?;
    globals.set(WRAPPER_CACHE, ctx.eval::<Value, _>("new Map()")?)?;

    let document = Class::instance(ctx.clone(), GosubDocument::new(doc))?;
    globals.set("document", document)?;
    globals.set("window", globals.clone())?;
    globals.set("self", globals.clone())?;
    Ok(())
}

/// Wrap `id` in its JS object, reusing the cached wrapper so node identity is stable.
pub(crate) fn wrap<'js>(
    ctx: &Ctx<'js>,
    doc: &DocHandle,
    id: gosub_shared::node::NodeId,
) -> rquickjs::Result<Value<'js>> {
    let cache: Object<'js> = ctx.globals().get(WRAPPER_CACHE)?;
    let key = id.as_usize() as f64;

    let existing: Value<'js> = cache
        .get::<_, rquickjs::Function>("get")?
        .call((rquickjs::function::This(cache.clone()), key))?;
    if !existing.is_undefined() {
        return Ok(existing);
    }

    let wrapper = Class::instance(ctx.clone(), GosubNode::new(doc.clone(), id))?.into_value();
    cache.get::<_, rquickjs::Function>("set")?.call::<_, ()>((
        rquickjs::function::This(cache),
        key,
        wrapper.clone(),
    ))?;
    Ok(wrapper)
}

/// `wrap`, but `None` becomes JS `null` - the DOM's answer for "no such node".
pub(crate) fn wrap_opt<'js>(
    ctx: &Ctx<'js>,
    doc: &DocHandle,
    id: Option<gosub_shared::node::NodeId>,
) -> rquickjs::Result<Value<'js>> {
    match id {
        Some(id) => wrap(ctx, doc, id),
        None => Ok(Value::new_null(ctx.clone())),
    }
}
