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
use gosub_interface::font_system::FontSystem;
use gosub_interface::render::backend::{CompositorSink, RenderBackend};
use gosub_fontmanager::ParleyFontSystem;
use gosub_render_pipeline::render::backends::null::NullBackend;
use gosub_render_pipeline::render::DefaultCompositor;
use gosub_shared::node::NodeId;
use std::marker::PhantomData;

/// The engine's default config, wiring the gosub_html5 document implementation together with the
/// gosub_css3 style system, parameterized over the render backend `B`, font system `F`, and
/// compositor sink `S` — in that order, so the rarely-changed compositor falls off as a default.
///
/// Embedders that use the default parse stack pick a backend (and optionally a font system):
/// `DefaultConfig<CairoBackend, PangoFontSystem>`. With no parameters, `DefaultConfig` is the
/// headless `DefaultConfig<NullBackend, ParleyFontSystem, DefaultCompositor>`. Embedders that also
/// want a custom CSS/DOM/parser stack implement [`ModuleConfiguration`] + [`EngineConfig`] on their
/// own type instead.
#[allow(clippy::type_complexity)] // PhantomData marker carrying the three config type params
pub struct DefaultConfig<B = NullBackend, F = ParleyFontSystem, S = DefaultCompositor>(PhantomData<fn() -> (B, F, S)>);

// `DefaultConfig` is a zero-sized marker; its Clone/Debug/PartialEq are independent of `B`/`S`/`F`
// (which are never instantiated), so we impl them by hand rather than deriving bounds on them.
impl<B, F, S> Clone for DefaultConfig<B, F, S> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}
impl<B, F, S> std::fmt::Debug for DefaultConfig<B, F, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DefaultConfig")
    }
}
impl<B, F, S> PartialEq for DefaultConfig<B, F, S> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<B, F, S> ModuleConfiguration for DefaultConfig<B, F, S>
where
    B: RenderBackend + Send + Sync + 'static,
    S: CompositorSink + 'static,
    F: FontSystem + Default,
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
    /// Font system used for text measurement (layout) and shared with the renderer for drawing.
    /// The engine owns one instance, created via `Default`, and hands it to both.
    type FontSystem: FontSystem + Default;
}

impl<B, F, S> EngineConfig for DefaultConfig<B, F, S>
where
    B: RenderBackend + Send + Sync + 'static,
    S: CompositorSink + 'static,
    F: FontSystem + Default,
{
    type RenderBackend = B;
    type CompositorSink = S;
    type FontSystem = F;
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
