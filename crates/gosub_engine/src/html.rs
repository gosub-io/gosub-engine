//! HTML parsing and related utilities.
//!
//! This module provides functionality to parse HTML documents, extract resource hints,
//! and handle various HTML configurations.
mod parser;

pub use parser::parse_main_document_stream;
pub use parser::{DocumentError, DummyDocument, DummyHtml5Config, ResourceHint};

use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_interface::render::backend::{CompositorSink, RenderBackend};
use gosub_render_pipeline::render::backends::null::NullBackend;
use gosub_render_pipeline::render::DefaultCompositor;
use gosub_shared::node::NodeId;
use std::marker::PhantomData;

/// The engine's default config, wiring the gosub_html5 document implementation together with the
/// gosub_css3 style system, and parameterized over the render backend `B` and compositor sink `S`.
///
/// Embedders that use the default parse stack only need to pick a backend:
/// `GosubEngine::<DefaultConfig<CairoBackend>>::new(...)`. With no parameters, `DefaultConfig`
/// is the headless `DefaultConfig<NullBackend, DefaultCompositor>`. Embedders that also want a
/// custom CSS/DOM/parser stack implement [`ModuleConfiguration`] + [`EngineConfig`] on their own
/// type instead.
pub struct DefaultConfig<B = NullBackend, S = DefaultCompositor>(PhantomData<fn() -> (B, S)>);

// `DefaultConfig` is a zero-sized marker; its Clone/Debug/PartialEq are independent of `B`/`S`
// (which are never instantiated), so we impl them by hand rather than deriving bounds on B/S.
impl<B, S> Clone for DefaultConfig<B, S> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}
impl<B, S> std::fmt::Debug for DefaultConfig<B, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DefaultConfig")
    }
}
impl<B, S> PartialEq for DefaultConfig<B, S> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<B, S> ModuleConfiguration for DefaultConfig<B, S>
where
    B: RenderBackend + Send + Sync + 'static,
    S: CompositorSink + 'static,
{
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

/// A [`ModuleConfiguration`] this engine can actually drive: it pins `Document = DocumentImpl<Self>`
/// (the HTML parser produces that concrete type) and names the runtime render components.
///
/// `RenderBackend`/`CompositorSink` live here rather than on `ModuleConfiguration` so that
/// parse-only configs (parser test harnesses, fuzz targets) — which never render and must not
/// depend on the renderer crates — only implement `ModuleConfiguration`. Engine code bounds on
/// `C: EngineConfig`; the public `ModuleConfiguration` stays render-agnostic.
pub trait EngineConfig: ModuleConfiguration<Document = DocumentImpl<Self>> {
    /// Low-level render backend (Cairo, Skia, Vello, null, …).
    type RenderBackend: RenderBackend + Send + Sync;
    /// Receives finished frames from the render backend.
    type CompositorSink: CompositorSink;
}

impl<B, S> EngineConfig for DefaultConfig<B, S>
where
    B: RenderBackend + Send + Sync + 'static,
    S: CompositorSink + 'static,
{
    type RenderBackend = B;
    type CompositorSink = S;
}

/// The parsed document type used by the engine for a given config (defaults to [`DefaultConfig`]).
pub type EngineDocument<C = DefaultConfig> = DocumentImpl<C>;

/// Extract the text content of the first `<title>` element in the document.
pub fn document_title<C: EngineConfig>(doc: &EngineDocument<C>) -> Option<String> {
    find_title(doc, doc.root())
}

fn find_title<C: EngineConfig>(doc: &EngineDocument<C>, node_id: NodeId) -> Option<String> {
    for &child in doc.children(node_id) {
        if doc.node_type(child) == NodeType::ElementNode {
            if doc
                .tag_name(child)
                .map(|t| t.eq_ignore_ascii_case("title"))
                .unwrap_or(false)
            {
                // Collect text from title's children
                let mut text = String::new();
                for &t in doc.children(child) {
                    if doc.node_type(t) == NodeType::TextNode {
                        if let Some(v) = doc.text_value(t) {
                            text.push_str(v);
                        }
                    }
                }
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            } else if let Some(found) = find_title(doc, child) {
                return Some(found);
            }
        }
    }
    None
}
