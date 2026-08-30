//! HTML parsing entry points and the render-side configuration traits.
mod parser;
pub(crate) mod web_fonts;

pub use parser::parse_main_document_stream;
pub use parser::{DocumentError, HtmlParseConfig, ResourceHint};

use gosub_css3::system::Css3System;
use gosub_fontmanager::ParleyFontSystem;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::document::Document as _;
use gosub_interface::font_system::FontSystem;
use gosub_interface::node::NodeType;
use gosub_interface::render::backend::{CompositorSink, RenderBackend};
use gosub_render_pipeline::render::backends::null::NullBackend;
use gosub_render_pipeline::render::DefaultCompositor;
use gosub_shared::node::NodeId;
use std::marker::PhantomData;

/// The engine's default config, wiring the gosub_html5 document implementation together with the
/// gosub_css3 style system, parameterized over the render backend `B`, font system `F`, and
/// compositor sink `S` - in that order, so the rarely-changed compositor falls off as a default.
///
/// Embedders that use the default parse stack pick a backend (and optionally a font system):
/// `DefaultRenderConfig<CairoBackend, PangoFontSystem>`. With no parameters, `DefaultRenderConfig` is the
/// headless `DefaultRenderConfig<NullBackend, ParleyFontSystem, DefaultCompositor>`. Embedders that also
/// want a custom CSS/DOM/parser stack implement [`ModuleConfiguration`] + [`RenderConfiguration`] on their
/// own type instead.
#[allow(clippy::type_complexity)] // PhantomData marker carrying the three config type params
pub struct DefaultRenderConfig<B = NullBackend, F = ParleyFontSystem, S = DefaultCompositor>(
    PhantomData<fn() -> (B, F, S)>,
);

// `DefaultRenderConfig` is a zero-sized marker; its Clone/Debug/PartialEq are independent of `B`/`S`/`F`
// (which are never instantiated), so we impl them by hand rather than deriving bounds on them.
impl<B, F, S> Clone for DefaultRenderConfig<B, F, S> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}
impl<B, F, S> std::fmt::Debug for DefaultRenderConfig<B, F, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DefaultRenderConfig")
    }
}
impl<B, F, S> PartialEq for DefaultRenderConfig<B, F, S> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<B, F, S> ModuleConfiguration for DefaultRenderConfig<B, F, S>
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
/// parse-only configs (parser test harnesses, fuzz targets) - which never render and must not
/// depend on the renderer crates - only implement `ModuleConfiguration`. Engine code bounds on
/// `C: RenderConfiguration`; the public `ModuleConfiguration` stays render-agnostic.
pub trait RenderConfiguration: ModuleConfiguration<Document = DocumentImpl<Self>> {
    /// Low-level render backend (Cairo, Skia, Vello, null, …).
    type RenderBackend: RenderBackend + Send + Sync;
    /// Receives finished frames from the render backend.
    type CompositorSink: CompositorSink;
    /// Font system used for text measurement (layout) and shared with the renderer for drawing.
    /// The engine owns one instance, created via `Default`, and hands it to both.
    type FontSystem: FontSystem + Default;

    /// A stage-6 tile rasterizer for a forked renderer process, or `None`
    /// if forked renderers should stop after painting.
    fn forked_tile_rasterizer(
        font_system: std::sync::Arc<parking_lot::Mutex<dyn gosub_interface::font_system::FontSystem>>,
    ) -> Option<Box<dyn gosub_render_pipeline::rasterizer::Rasterable + Send + Sync>> {
        let _ = font_system;
        None
    }
}

impl<B, F, S> RenderConfiguration for DefaultRenderConfig<B, F, S>
where
    B: RenderBackend + Send + Sync + 'static,
    S: CompositorSink + 'static,
    F: FontSystem + Default,
{
    type RenderBackend = B;
    type CompositorSink = S;
    type FontSystem = F;

    /// A CPU tile rasterizer for forked renderers, when one is compiled in
    /// (`cairo-tiles`, else `skia-tiles`). Independent of `B`: a GPU backend in
    /// the broker still receives isolated tiles as CPU pixels.
    fn forked_tile_rasterizer(
        font_system: std::sync::Arc<parking_lot::Mutex<dyn gosub_interface::font_system::FontSystem>>,
    ) -> Option<Box<dyn gosub_render_pipeline::rasterizer::Rasterable + Send + Sync>> {
        #[cfg(feature = "cairo-tiles")]
        {
            Some(Box::new(gosub_renderer_cairo::CairoRasterizer::with_font_system(
                font_system,
            )))
        }
        #[cfg(all(feature = "skia-tiles", not(feature = "cairo-tiles")))]
        {
            Some(Box::new(gosub_renderer_skia::SkiaRasterizer::with_font_system(
                1.0,
                font_system,
            )))
        }
        #[cfg(not(any(feature = "cairo-tiles", feature = "skia-tiles")))]
        {
            let _ = font_system;
            None
        }
    }
}

/// The parsed document type used by the engine for a given config (defaults to [`DefaultRenderConfig`]).
pub type EngineDocument<C = DefaultRenderConfig> = DocumentImpl<C>;

/// Extract the text content of the first `<title>` element in the document.
pub fn document_title<C: RenderConfiguration>(doc: &EngineDocument<C>) -> Option<String> {
    find_title(doc, doc.root())
}

/// Whether `node_id` is a text-editable control: `<textarea>`, a text-like
/// `<input>`, or `contenteditable`.
pub fn is_text_input<C: RenderConfiguration>(doc: &EngineDocument<C>, node_id: NodeId) -> bool {
    match doc.tag_name(node_id) {
        Some("textarea") => true,
        Some("input") => !doc.attribute(node_id, "type").is_some_and(|t| {
            [
                "button", "submit", "reset", "checkbox", "radio", "range", "color", "file", "image", "hidden",
            ]
            .iter()
            .any(|k| t.eq_ignore_ascii_case(k))
        }),
        _ => doc
            .attribute(node_id, "contenteditable")
            .is_some_and(|v| v.is_empty() || v.eq_ignore_ascii_case("true")),
    }
}

/// The document's icon: the first `<link rel="icon">` (or `shortcut icon`,
/// `apple-touch-icon*`) resolved against `base_url`, else `/favicon.ico` for
/// http(s) documents.
pub fn favicon_url<C: RenderConfiguration>(doc: &EngineDocument<C>, base_url: &url::Url) -> Option<url::Url> {
    fn walk<C: RenderConfiguration>(doc: &EngineDocument<C>, node: NodeId, base: &url::Url) -> Option<url::Url> {
        for &child in doc.children(node) {
            if doc.tag_name(child).is_some_and(|t| t.eq_ignore_ascii_case("link")) {
                let is_icon = doc.attribute(child, "rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace().any(|t| {
                        t.eq_ignore_ascii_case("icon")
                            || t.len() >= 16 && t[..16].eq_ignore_ascii_case("apple-touch-icon")
                    })
                });
                if is_icon {
                    if let Some(url) = doc.attribute(child, "href").and_then(|h| base.join(h).ok()) {
                        return Some(url);
                    }
                }
            }
            if let Some(found) = walk::<C>(doc, child, base) {
                return Some(found);
            }
        }
        None
    }

    walk::<C>(doc, doc.root(), base_url).or_else(|| {
        matches!(base_url.scheme(), "http" | "https")
            .then(|| base_url.join("/favicon.ico").ok())
            .flatten()
    })
}

fn find_title<C: RenderConfiguration>(doc: &EngineDocument<C>, node_id: NodeId) -> Option<String> {
    for &child in doc.children(node_id) {
        if doc.node_type(child) != NodeType::ElementNode {
            continue;
        }

        let is_title = doc.tag_name(child).is_some_and(|t| t.eq_ignore_ascii_case("title"));
        if !is_title {
            if let Some(found) = find_title(doc, child) {
                return Some(found);
            }
            continue;
        }

        // Collect text from title's children
        let mut text = String::new();
        for &t in doc.children(child) {
            if doc.node_type(t) != NodeType::TextNode {
                continue;
            }
            if let Some(v) = doc.text_value(t) {
                text.push_str(v);
            }
        }
        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
        // Empty <title>: keep scanning siblings (an empty title is not recursed into).
    }
    None
}
