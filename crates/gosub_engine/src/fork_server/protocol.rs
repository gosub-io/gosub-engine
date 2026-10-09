//! The broker↔fork-server wire vocabulary.

use crate::net::types::ResourceKind;
use serde::{Deserialize, Serialize};

/// The confinement answer, as it crosses the process boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfinementTier {
    /// The font system front-loaded everything; renderers get the strictest
    /// sandbox (no file access at all).
    Full,
    /// The font system reads font files while operating; renderers get
    /// read-only font paths plus a private writable scratch.
    FontPathsReadable,
    /// The font system cannot run isolated; the fork server refuses to fork
    /// and the engine must render single-process.
    Unsupported(String),
}

impl From<&gosub_interface::font_system::Confinement> for ConfinementTier {
    fn from(answer: &gosub_interface::font_system::Confinement) -> Self {
        use gosub_interface::font_system::Confinement;
        match answer {
            Confinement::Full => ConfinementTier::Full,
            Confinement::FontPathsReadable => ConfinementTier::FontPathsReadable,
            Confinement::Unsupported(reason) => ConfinementTier::Unsupported(reason.clone()),
        }
    }
}

/// Broker → fork server.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToForkServer {
    /// Liveness check.
    Ping,
    /// Fork a renderer, confine it to the announced tier, shape text with the
    /// inherited (copy-on-write) font system, and report the measured box.
    ForkProof,
    /// Fork a renderer and run the render pipeline in it: parse `html`, style,
    /// lay out against the viewport, layer, tile, and paint - single-threaded,
    /// under the announced tier's sandbox, measuring and shaping through the
    /// inherited font system. Replies with [`FromForkServer::PageRendered`].
    RenderPage {
        html: String,
        /// The page's URL - the base against which the renderer resolves
        /// relative subresource URLs (stylesheets, images, fonts).
        url: String,
        /// The tab this render is for. Display only (telemetry, logs); the
        /// process name never carries it. Empty when the caller has no tab.
        tab: String,
        viewport_width: f64,
        viewport_height: f64,
        /// The broker's device-pixel ratio. Tile pixels are physical; the
        /// renderer is another process, so the host's global does not reach it.
        dpr: u32,
        /// The user's preferences the page's `@media` queries and
        /// `light-dark()` answer to - process-wide state in the broker, so
        /// they travel the same way as `dpr`.
        media: MediaPrefs,
        /// Content hashes of tiles the broker still holds from a previous
        /// render of this tab. A tile whose hash is in here is neither
        /// rasterized nor shipped - the renderer answers
        /// [`TileUnchanged`](FromForkServer::TileUnchanged) and the broker
        /// reuses the pixels it already has. Empty on a first render.
        known_tiles: Vec<u64>,
        /// The DOM node under the pointer, so the renderer can apply
        /// `:hover` styles. The broker hit-tests (it holds the geometry and
        /// its own document) and tells the renderer the answer; the renderer
        /// re-parses per render and would otherwise have no hover state at
        /// all. With `known_tiles`, a hover re-render costs only the tiles
        /// whose painted content actually changed.
        hovered_node: Option<u64>,
    },
    /// Fork a *resident* renderer - one that stays alive and serves
    /// [`ToRenderer`] requests until told to stop - confine it to the announced
    /// tier, and hand its end of a private link to the broker: the reply is
    /// [`FromForkServer::RendererSpawned`] followed immediately by the link's
    /// file descriptor. From then on the broker and that renderer talk
    /// directly; the fork server is out of the loop.
    SpawnRenderer {
        /// The pool's key for this renderer (zone + site). Display only; the
        /// process names itself `renderer-<id>`.
        label: String,
    },
    /// Collect any resident renderers that have exited (they are this
    /// process's children, so only it can reap them). The broker sends this
    /// after it observed a renderer's link close.
    ReapExited,
    /// Run the escape audit in the fork server itself.
    Audit,
    /// Fork a renderer that runs the escape audit and reports it.
    AuditRenderer,
    /// Exit cleanly.
    Shutdown,
    /// The broker's answer to [`FromForkServer::NeedResource`], relayed on to
    /// the renderer that is blocked waiting for it. Only ever sent while a
    /// [`RenderPage`](ToForkServer::RenderPage) exchange is in flight.
    Resource(ResourceReply),
}

/// Broker → resident renderer, over the private link
/// [`ToForkServer::SpawnRenderer`] handed over. One renderer hosts every tab
/// of one (zone, site), so requests name their tab.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToRenderer {
    /// A tab now lives in this renderer.
    OpenTab { tab: String },
    /// The tab left (closed, or moved to another site's renderer).
    CloseTab { tab: String },
    /// Render this tab's page: the same one-shot pipeline as
    /// [`ToForkServer::RenderPage`], answered with the same streamed
    /// [`FromRenderer`] sequence ending in [`FromRenderer::Rendered`]. A
    /// [`FromRenderer::NeedResource`] mid-render is answered with a bare
    /// [`ResourceReply`] frame (the renderer's loader reads exactly that),
    /// so the link strictly alternates for the whole exchange.
    Navigate {
        tab: String,
        html: String,
        url: String,
        viewport_width: f64,
        viewport_height: f64,
        /// See [`ToForkServer::RenderPage::dpr`]; a retained page keeps it
        /// until the next navigate.
        dpr: u32,
        /// See [`ToForkServer::RenderPage::media`]; kept the same way.
        media: MediaPrefs,
        /// Where the viewport is: only the raster window around it is
        /// rasterized and shipped (see [`ToRenderer::Scroll`]).
        scroll_y: f64,
        known_tiles: Vec<u64>,
        hovered_node: Option<u64>,
    },
    /// The viewport moved on a page this renderer retains: rasterize what
    /// came into the raster window and ship it, and announce
    /// ([`FromRenderer::Evict`]) tiles that drifted too far to keep. Answered
    /// with the same streamed sequence as `Navigate`; the summary's tile
    /// counts cover this pass only.
    Scroll { tab: String, scroll_y: f64 },
    /// The pointer moved to `node` (a DOM node id the broker hit-tested, or
    /// nothing) on a page this renderer retains: restyle the hover chains and
    /// repaint just the tiles they cover. Same streamed answer as `Scroll`.
    Hover { tab: String, node: Option<u64> },
    /// The user acted on a page this renderer retains: apply the event to
    /// the page's DOM, re-lay out when the boxes may have moved, and answer
    /// with the same streamed sequence as `Scroll` - the tiles the input
    /// changed - ending in a [`FromRenderer::Rendered`] that carries what
    /// the input asked of the broker ([`Effect`]). `known_tiles` is what the
    /// broker holds, as on `Navigate`: a re-layout ships the page again by
    /// content hash, and only tiles whose pixels changed travel.
    Input {
        tab: String,
        /// Where the viewport is; viewport coordinates in `event` are
        /// measured against it.
        scroll_y: f64,
        known_tiles: Vec<u64>,
        event: InputEvent,
    },
    /// The viewport of a page this renderer retains changed size (or device
    /// pixel ratio): lay the retained document out again at the new size,
    /// which re-evaluates width-dependent `@media` rules, and answer like an
    /// input that re-laid the page out - the window shipped by content hash
    /// against `known_tiles`, what the broker held and this layout no longer
    /// accounts for evicted, the geometry for hit tests. No parse, no fetch.
    Resize {
        tab: String,
        viewport_width: f64,
        viewport_height: f64,
        dpr: u32,
        scroll_y: f64,
        known_tiles: Vec<u64>,
    },
    /// Die without replying, the way a crashing renderer would. For tests
    /// of the broker's recovery; a renderer that obeys it was going to be
    /// trusted with nothing anyway.
    CrashForTest,
    /// Run the escape audit in this process and report it.
    Audit,
    /// Exit cleanly. The broker sends this when the renderer's last tab
    /// closes; a closed link means the same.
    Shutdown,
}

/// One user action on a retained page, in viewport CSS px - the space the
/// embedder's pointer events arrive in. The host's scroll offset travels
/// beside it on [`ToRenderer::Input`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputEvent {
    PointerDown {
        x: f64,
        y: f64,
        button: crate::engine::events::MouseButton,
    },
    PointerUp {
        x: f64,
        y: f64,
        button: crate::engine::events::MouseButton,
    },
    /// Only while the renderer holds a pointer capture (see
    /// [`Effect::Capture`]); a plain move is hover, which stays
    /// [`ToRenderer::Hover`].
    PointerMove {
        x: f64,
        y: f64,
    },
    /// A wheel notch over the page: a dropdown or a textarea may take it;
    /// otherwise the broker scrolls the page itself.
    Wheel {
        x: f64,
        y: f64,
        delta_y: f64,
    },
    KeyDown {
        key: String,
        /// [`Modifiers`](crate::engine::events::Modifiers) as bits.
        modifiers: u8,
    },
    KeyUp {
        key: String,
        modifiers: u8,
    },
    /// Committed text for the focused control: IME output, or the clipboard
    /// in answer to [`Effect::PasteRequested`].
    Text {
        text: String,
    },
    /// The embedder's picker moved; see [`Effect::Picker`].
    PickerChanged {
        value: String,
    },
    PickerClosed,
    /// The window lost focus: blur, end gestures, close popups.
    Blur,
}

/// A rectangle in viewport CSS px, as an effect reports it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What an input pass asks of the broker. A request, every one of it: the
/// renderer is the process a page exploits, and the broker checks each
/// before acting, as it does a hit region's link.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Effect {
    /// Keyboard focus moved or cleared. `bounds` is the focused control's
    /// border box, for IME placement and scrolling it into view.
    Focus {
        focused: bool,
        editable: bool,
        bounds: Option<WireRect>,
    },
    /// The cursor for the pointer's position after the event.
    Cursor { cursor: HitCursor },
    /// A form submission, or a link activated from the keyboard or by a
    /// click the renderer saw first: a navigation for the broker to decide
    /// on. `body` is the form-encoded body of a POST.
    Navigate {
        url: String,
        post: bool,
        body: Option<String>,
    },
    /// A control that needs the embedder's picker: the fields of
    /// `EngineEvent::PickerRequested`.
    Picker {
        kind: crate::engine::events::PickerKind,
        bounds: WireRect,
        value: String,
        min: Option<String>,
        max: Option<String>,
        step: Option<String>,
    },
    /// The page copied or cut `text` in a text control.
    ClipboardWrite { text: String },
    /// The page wants to paste: the clipboard comes back as
    /// [`InputEvent::Text`].
    PasteRequested,
    /// The renderer is mid-gesture (a slider thumb, a textarea grip, a
    /// dropdown scrollbar, a selection drag) or has let go: while held, send
    /// it pointer moves and wheel and skip hover processing.
    Capture { pointer: bool },
}

/// Effects one pass may carry. A handful answer any one event; past this
/// the broker treats the frame as a renderer gone wrong.
pub const MAX_EFFECTS: usize = 16;

/// Fork server → broker.
#[derive(Debug, Serialize, Deserialize)]
pub enum FromForkServer {
    /// Sent once, after the font system answered and the fork server confined
    /// itself accordingly: it is warmed, sandboxed, and ready to fork.
    Ready { tier: ConfinementTier },
    /// Liveness reply.
    Pong,
    /// A forked renderer shaped text under its tier sandbox and measured this.
    Proof { width: f32, height: f32 },
    /// One rasterized tile of the page being rendered; its sealed-memfd file
    /// descriptor follows immediately on the link. Streamed: tiles arrive
    /// one at a time, each fd relayed and released before the next - no side
    /// of the transport ever holds more than one tile fd, so page size is
    /// bounded by memory, not by file-descriptor limits.
    Tile(TileHeader),
    /// A tile the broker already holds (its hash was in `known_tiles`): no
    /// fd follows and nothing was rasterized for it. Arrives in composite
    /// order alongside `Tile`, so the broker rebuilds the page's tile list
    /// by walking the two in the order they came.
    TileUnchanged(TileHeader),
    /// The render finished; every [`Tile`](FromForkServer::Tile) of the page
    /// has already streamed past. A render that dies mid-stream ends in
    /// [`Refused`](FromForkServer::Refused) instead, and the broker discards
    /// the partial tile set - atomicity lives at the consumer now, not in
    /// transport buffering.
    PageRendered {
        summary: PageSummary,
        hit_regions: Vec<HitRegion>,
    },
    /// A resident renderer was forked; the broker's end of its link follows
    /// immediately as a file descriptor. `pid` is the number the fork
    /// server's own namespace - and so the broker's - sees.
    RendererSpawned { pid: i32 },
    /// Answer to [`ToForkServer::Audit`] and [`ToForkServer::AuditRenderer`].
    /// Only with the sandbox compiled in: the rest of this protocol is plain
    /// data that builds everywhere.
    #[cfg(feature = "process-isolation")]
    Audit(gosub_sandbox::audit::AuditReport),
    /// The request could not be served; the string says why (e.g. forking is
    /// refused under `Unsupported`, or the forked child died).
    Refused(String),
    /// A renderer needs a subresource it has no capability to fetch - the
    /// brokered-load inversion, mirroring cookies: the renderer names what it
    /// wants, the broker performs the fetch where identity and cookies live,
    /// and only bytes come back. Sent mid-[`RenderPage`](ToForkServer::RenderPage);
    /// the broker answers with [`ToForkServer::Resource`] before anything else.
    NeedResource {
        url: String,
        /// What the renderer will use it as, which decides the mixed-content handling. Not
        /// checked: a renderer claiming an image for a stylesheet gets it upgraded rather than
        /// refused, and neither sends anything over plain `http`.
        kind: ResourceKind,
        deferred: bool,
    },
}

/// What a forked renderer sends its parent over their private pair before
/// exiting. Internal to the fork-server process family, but it crosses a
/// process boundary (fork), so it is wire vocabulary all the same.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProofReply {
    pub width: f32,
    pub height: f32,
}

/// What a page came to, measured by the forked renderer that laid it out and
/// painted it. Numbers rather than pixels - the pixels travel separately, as
/// sealed memfds - but enough on their own for the broker to assert the
/// pipeline really ran (a dead font system collapses heights to zero; a dead
/// painter produces no commands).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageSummary {
    /// The document's `<title>` and icon URL, from the renderer's parse: the
    /// broker does not parse page content itself.
    pub title: Option<String>,
    pub favicon: Option<String>,
    pub page_width: f64,
    pub page_height: f64,
    pub layer_count: u64,
    pub painted_tiles: u64,
    pub paint_commands: u64,
    /// Layer ids back to front. A broker holding tiles from several passes
    /// (a retained page scrolled about) composites them in this order; the
    /// renderer's tile list is the only thing that knows it.
    pub layer_order: Vec<u64>,
    /// What the renderer spent on this pass, per stage, in microseconds. It
    /// has no way to report anywhere itself; the broker relays these to the
    /// telemetry firehose on its behalf.
    pub timings_us: Vec<(String, u64)>,
    /// Where the page's `#fragment` targets are: the broker keeps no layout
    /// of a remotely rendered page, so navigating to `#section` looks here.
    pub fragment_targets: Vec<FragmentTarget>,
    /// The renderer has no page retained for this tab (replaced after a
    /// crash, or past its retained-page limit), so this pass rendered
    /// nothing. Said outright: an empty answer from a retained page - a blank
    /// page lays out 0px tall - looks the same otherwise.
    pub no_page: bool,
}

/// An element a `#fragment` can scroll to, in layout order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FragmentTarget {
    /// The `id`, or the `name` of an `<a name>`.
    pub name: String,
    /// Whether `name` is an `id`: an `id` match wins over any `<a name>`.
    pub by_id: bool,
    /// The top of the element's border box, in page space.
    pub y: f64,
}

/// The most fragment targets a renderer sends, or the broker keeps. Local
/// lookups are not capped.
pub const MAX_FRAGMENT_TARGETS: usize = 20_000;

/// Where `name` scrolls to: the first `id` target with that name, else the
/// first `<a name>` one - the HTML spec's order of lookup.
pub fn find_fragment_target(targets: &[FragmentTarget], name: &str) -> Option<f64> {
    let first = |by_id: bool| targets.iter().find(|t| t.by_id == by_id && t.name == name);
    first(true).or_else(|| first(false)).map(|t| t.y)
}

/// One hit-testable box of the page, in page space, in hit-test order:
/// the first region containing a point is the one under the pointer (the
/// renderer emits them exactly as its layer list would have walked them,
/// topmost first).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// The node in the renderer's DOM; only meaningful back in that renderer
    /// (`Hover { node }`).
    pub node_id: u64,
    /// How the region's layer responds to scroll; the broker inverts the same
    /// composite mapping the tiles use.
    pub anchor: TileWireAnchor,
    /// What the broker would otherwise read off a DOM it no longer holds:
    /// the nearest enclosing `<a href>` (absolute), the `<img src>` at or
    /// around the box, the cursor for the box, and whether it is editable.
    pub link: Option<String>,
    pub image: Option<String>,
    pub cursor: HitCursor,
    pub editable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HitCursor {
    #[default]
    Default,
    Pointer,
    Text,
    /// Over a textarea's resize grip; only an input pass reports it.
    Resize,
}

/// Upper bound on regions shipped for one page. A pathological page (tens of
/// thousands of boxes) would otherwise spend more on hit geometry than on
/// pixels; past this the tail is dropped and the renderer says so, rather
/// than silently pretending the page ends there.
pub const MAX_HIT_REGIONS: usize = 20_000;

/// Longest `link`/`image` string a hit region carries, and longest favicon
/// URL. A longer one is dropped whole on both sides, never cut: a cut URL
/// would be navigated to.
pub const MAX_HIT_TEXT: usize = 2048;

/// Most bytes of `link`/`image` text one page's regions carry together. Every
/// box under an `<a>` repeats its href; past this the rest ship without
/// strings (the cursor still says pointer), so the `Rendered` frame stays
/// well inside the transport's frame cap instead of killing the renderer.
pub const MAX_HIT_TEXT_TOTAL: usize = 4 * 1024 * 1024;

/// The user preferences a render answers `@media` queries with, as the
/// broker holds them (`renderer.prefers_color_scheme`,
/// `renderer.prefers_reduced_motion`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaPrefs {
    pub prefers_dark: bool,
    pub prefers_reduced_motion: bool,
}

/// Everything about one rasterized tile except its pixels, which follow as a
/// sealed memfd (see `gosub_ipc::shm` - the consumer derives the byte count
/// from these dimensions and validates the fd against them, never trusting a
/// length from the wire). Carries what the compositor's `CachedTile` needs,
/// so a mapped tile converts without consulting the renderer again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileHeader {
    /// Position of this tile on the page, in CSS pixels.
    pub page_x: f64,
    pub page_y: f64,
    /// Owning layer: `(0,0)` can exist in both a base layer and a sticky one.
    pub layer_id: u64,
    pub width: u32,
    pub height: u32,
    pub format: TileWireFormat,
    /// This tile's content hash (see
    /// `gosub_render_pipeline::rasterizer::tile_content_hash`): what the
    /// broker keys its kept pixels by, and what a later render compares
    /// against to decide the tile is unchanged.
    pub content_hash: u64,
    /// Group opacity of the tile's layer, applied by the compositor.
    pub opacity: f32,
    /// How the tile's layer responds to scroll.
    pub anchor: TileWireAnchor,
}

/// The in-memory byte order of a shipped tile - the wire mirror of the
/// interface crate's `PixelFormat` (which carries no serde).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileWireFormat {
    /// Little-endian premultiplied ARGB32 (`[B, G, R, A]`): Cairo, Skia.
    PreMulArgb32,
    /// Premultiplied RGBA8: Vello.
    Rgba8,
}

impl From<gosub_interface::render::backend::PixelFormat> for TileWireFormat {
    fn from(format: gosub_interface::render::backend::PixelFormat) -> Self {
        use gosub_interface::render::backend::PixelFormat;
        match format {
            PixelFormat::PreMulArgb32 => TileWireFormat::PreMulArgb32,
            PixelFormat::Rgba8 => TileWireFormat::Rgba8,
        }
    }
}

impl From<TileWireFormat> for gosub_interface::render::backend::PixelFormat {
    fn from(format: TileWireFormat) -> Self {
        use gosub_interface::render::backend::PixelFormat;
        match format {
            TileWireFormat::PreMulArgb32 => PixelFormat::PreMulArgb32,
            TileWireFormat::Rgba8 => PixelFormat::Rgba8,
        }
    }
}

/// Wire mirror of the interface crate's `TileAnchor` (no serde there).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TileWireAnchor {
    Scroll,
    Fixed,
    Sticky(StickyWire),
}

/// Wire mirror of `StickyConstraint`: plain page-space geometry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StickyWire {
    pub inset_top: Option<f64>,
    pub inset_left: Option<f64>,
    pub natural_x: f64,
    pub natural_y: f64,
    pub natural_w: f64,
    pub natural_h: f64,
    pub cage_x: f64,
    pub cage_y: f64,
    pub cage_w: f64,
    pub cage_h: f64,
}

impl From<gosub_interface::render::backend::TileAnchor> for TileWireAnchor {
    fn from(anchor: gosub_interface::render::backend::TileAnchor) -> Self {
        use gosub_interface::render::backend::TileAnchor;
        match anchor {
            TileAnchor::Scroll => TileWireAnchor::Scroll,
            TileAnchor::Fixed => TileWireAnchor::Fixed,
            TileAnchor::Sticky(s) => TileWireAnchor::Sticky(StickyWire {
                inset_top: s.inset_top,
                inset_left: s.inset_left,
                natural_x: s.natural_x,
                natural_y: s.natural_y,
                natural_w: s.natural_w,
                natural_h: s.natural_h,
                cage_x: s.cage_x,
                cage_y: s.cage_y,
                cage_w: s.cage_w,
                cage_h: s.cage_h,
            }),
        }
    }
}

impl From<TileWireAnchor> for gosub_interface::render::backend::TileAnchor {
    fn from(anchor: TileWireAnchor) -> Self {
        use gosub_interface::render::backend::{StickyConstraint, TileAnchor};
        match anchor {
            TileWireAnchor::Scroll => TileAnchor::Scroll,
            TileWireAnchor::Fixed => TileAnchor::Fixed,
            TileWireAnchor::Sticky(s) => TileAnchor::Sticky(StickyConstraint {
                inset_top: s.inset_top,
                inset_left: s.inset_left,
                natural_x: s.natural_x,
                natural_y: s.natural_y,
                natural_w: s.natural_w,
                natural_h: s.natural_h,
                cage_x: s.cage_x,
                cage_y: s.cage_y,
                cage_w: s.cage_w,
                cage_h: s.cage_h,
            }),
        }
    }
}

/// Everything a renderer can say over its private link - to the fork server
/// (one-shot renderers) or straight to the broker (resident ones).
#[derive(Debug, Serialize, Deserialize)]
pub enum FromRenderer {
    /// Mid-render: fetch this for me. The parent relays it to the broker and
    /// sends the [`ResourceReply`] back; the renderer is blocked until then.
    /// With `deferred`, the renderer can do without it for now: the broker
    /// answers at once - the bytes if it has them, [`ResourceReply::Pending`]
    /// otherwise - and fetches in the background, re-rendering the tab when
    /// they land. Images ask this way; stylesheets and fonts, which layout
    /// cannot proceed without, do not.
    NeedResource {
        url: String,
        /// What the renderer will use it as, which decides the mixed-content handling. Not
        /// checked: a renderer claiming an image for a stylesheet gets it upgraded rather than
        /// refused, and neither sends anything over plain `http`.
        kind: ResourceKind,
        deferred: bool,
    },
    /// One rasterized tile; its sealed memfd follows immediately. The
    /// renderer seals, sends, and drops each before baking the next into a
    /// memfd, so it never holds more than one tile fd itself.
    Tile(TileHeader),
    /// A tile the broker already holds - no fd, no rasterization.
    TileUnchanged(TileHeader),
    /// Tiles the broker holds that the renderer will no longer account for:
    /// they drifted too far from the viewport of a retained page. The broker
    /// drops them; scrolling back there ships them afresh. Only a resident
    /// renderer sends this.
    Evict { hashes: Vec<u64> },
    /// Answer to [`ToRenderer::Audit`]. Only with the sandbox compiled in.
    #[cfg(feature = "process-isolation")]
    Audit(gosub_sandbox::audit::AuditReport),
    /// The final message: the render is complete, with the page's hit-test
    /// geometry and, after an [`ToRenderer::Input`], what the input asked of
    /// the broker. Hit regions travel with a navigate and with an input pass
    /// that laid the page out again; empty otherwise.
    Rendered {
        summary: PageSummary,
        hit_regions: Vec<HitRegion>,
        effects: Vec<Effect>,
    },
}

/// A fetched subresource (or its failure), as it travels broker → fork server
/// → renderer. Mirrors `crate::net::resource_loader::LoadedResource`,
/// which carries no serde.
#[derive(Debug, Serialize, Deserialize)]
pub enum ResourceReply {
    Ok {
        status: u16,
        content_type: Option<String>,
        body: Vec<u8>,
    },
    Failed(String),
    /// A deferred request the broker is still fetching; render without it.
    Pending,
    /// [`ResourceReply::Ok`] for a body too large for one frame: the body follows
    /// as a sealed memfd of `len` bytes (`gosub_ipc::shm::create_sealed_blob`), the
    /// next thing on the link - the tile channel in reverse.
    Shared {
        status: u16,
        content_type: Option<String>,
        len: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, by_id: bool, y: f64) -> FragmentTarget {
        FragmentTarget {
            name: name.into(),
            by_id,
            y,
        }
    }

    /// An `id` match wins over an `<a name>` one even when the anchor comes
    /// first; among equals, the first in the list (document order) wins.
    #[test]
    fn an_id_beats_an_anchor_name_and_the_first_match_wins() {
        let targets = [
            target("x", false, 10.0),
            target("x", true, 20.0),
            target("x", true, 30.0),
            target("y", false, 40.0),
        ];
        assert_eq!(find_fragment_target(&targets, "x"), Some(20.0));
        assert_eq!(find_fragment_target(&targets, "y"), Some(40.0));
        assert_eq!(find_fragment_target(&targets, "z"), None);
    }
}
