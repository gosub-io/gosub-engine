use cow_utils::CowUtils;

use crate::common::document::node::{Node, NodeId as DomNodeId, NodeType};
use crate::common::document::pipeline_doc::BgSize;
use crate::common::document::style::Display as CssDisplay;
use crate::common::document::style::{lookup, FontWeight, StyleProperty, TextAlign, Unit, Value};
use crate::common::font::{FontAlignment, FontInfo};
use crate::common::geo;
use crate::common::geo::Coordinate;
use crate::common::media::MediaStore;
use crate::common::media::{Media, MediaId, MediaRequest, MediaType};
use crate::layouter::abspos::{post_process_abspos, RebasedInsets};
use crate::layouter::box_model::Edges;
use crate::layouter::css_taffy_converter::CssTaffyConverter;
use crate::layouter::float::{
    float_side, position_is_out_of_flow, post_process_floats, resolve_bands_in_document_order, FloatBand,
};
use crate::layouter::table::post_process_tables;
use crate::layouter::text::get_text_layout;
use crate::layouter::{
    box_model, BackgroundMedia, CanLayout, ElementContext, ElementContextImage, ElementContextSvg, ElementContextText,
    LayoutElementId, LayoutElementNode, LayoutTree,
};
use crate::rendertree_builder::{RenderNodeId, RenderTree};
use gosub_fontmanager::ParleyFontSystem;
use gosub_interface::font_system::FontSystem;
use parking_lot::{Mutex, RwLock};
use std::borrow::Borrow;
use std::collections::HashMap;
use std::sync::Arc;
use taffy::prelude::*;
use taffy::NodeId as TaffyNodeId;

/// Whether a text node is nothing but the whitespace CSS collapses.
///
/// Deliberately ASCII: `str::trim` and `char::is_whitespace` use the Unicode set, which counts
/// U+00A0 and the other fixed-width spaces. Those are content - they exist to be kept - so a node
/// holding only an `&nbsp;` must not be mistaken for source indentation and dropped. Parsoid wraps
/// every entity in its own element, so that mistake cost Wikipedia's infoboxes every one of their
/// non-breaking spaces: "Designedby", "Firstappeared", "May1, 1964".
fn is_collapsible_whitespace(text: &str) -> bool {
    text.trim_matches(|c: char| c.is_ascii_whitespace()).is_empty()
}

/// Split text on the whitespace CSS actually collapses.
///
/// CSS `white-space` processing operates on spaces, tabs and newlines - not on every character
/// Unicode marks as white space. `str::split_whitespace` uses the Unicode set, which includes
/// U+00A0 NO-BREAK SPACE, so a text node holding only an `&nbsp;` split into *no* words at all and
/// was dropped. Parsoid wraps every entity in its own element, so Wikipedia's
/// `Designed<span typeof="mw:Entity">&nbsp;</span>by` rendered as "Designedby", and the same for
/// "First appeared", "May 1, 1964" and "; 62 years ago".
fn split_collapsible_whitespace(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_ascii_whitespace()).filter(|s| !s.is_empty())
}

const DEFAULT_FONT_SIZE: f64 = 16.0;

/// Width an inline item is measured at to get its natural size. Large enough that nothing wraps;
/// `f64::MAX` overflows inside the text stack, as the measure callback already notes.
const MAX_CONTENT_WIDTH: f64 = 1_000_000_000.0;

/// Whether an inline item is a run of whitespace, which is what the word splitter emits between
/// words and what CSS drops at a line break.
fn is_whitespace_item(layout_tree: &LayoutTree, id: &LayoutElementId) -> bool {
    match layout_tree.arena.get(id).map(|el| &el.context) {
        // `is_ascii_whitespace`, not `is_whitespace`: an `&nbsp;` is a character to render, not
        // a break opportunity, so it must not be dropped at a band boundary.
        Some(ElementContext::Text(text)) => {
            !text.text.is_empty() && text.text.chars().all(|c: char| c.is_ascii_whitespace())
        }
        _ => false,
    }
}

/// Length of `items` with any trailing whitespace items removed.
fn trim_trailing_whitespace(layout_tree: &LayoutTree, items: &[(LayoutElementId, TaffyNodeId)]) -> usize {
    let mut end = items.len();
    while end > 0 && is_whitespace_item(layout_tree, &items[end - 1].0) {
        end -= 1;
    }
    end
}

/// `items` with any leading whitespace items dropped.
fn skip_leading_whitespace<'a>(
    layout_tree: &LayoutTree,
    items: &'a [(LayoutElementId, TaffyNodeId)],
) -> &'a [(LayoutElementId, TaffyNodeId)] {
    let mut start = 0;
    while start < items.len() && is_whitespace_item(layout_tree, &items[start].0) {
        start += 1;
    }
    &items[start..]
}

/// How a block wants its line boxes laid out, beyond where they sit.
#[derive(Debug, Clone, Copy)]
struct LineStyle {
    /// The block's `text-align`, which positions runs that do not fill the line box.
    justify: Option<taffy::JustifyContent>,
    /// The block is a table cell: a flex column, where the axis that moves a line box across the
    /// cell is the cross axis. Its `align_items` does that, and an `align_self` on the line box
    /// would override it, so inside a cell the line box sets none.
    cell_aligned: bool,
}

/// Where one line box goes: which band it sits in, and how far below the previous one it starts.
#[derive(Debug, Clone, Copy)]
struct LinePlacement {
    band: FloatBand,
    offset_top: f32,
}

/// Walks a block's float bands as its line boxes are emitted, remembering how much of the current
/// band is already spoken for. One cursor covers a whole block, so a `<br>` in the middle of a
/// paragraph keeps its place in the band list rather than starting over.
struct BandCursor {
    bands: Vec<FloatBand>,
    index: usize,
    /// Height consumed in the band at `index`.
    used: f32,
    /// Height left unused when the previous band was left behind, to be added above the next
    /// container. A band 4.8 lines tall holds 4 whole lines; the fifth must start *below* the
    /// float rather than in the sliver left over, which is what CSS does with a line box that
    /// would otherwise intersect one.
    pending_offset: f32,
}

impl BandCursor {
    fn new(bands: &[FloatBand]) -> Self {
        Self {
            bands: bands.to_vec(),
            index: 0,
            used: 0.0,
            pending_offset: 0.0,
        }
    }

    /// The geometry for the next line box: the current band, plus any gap owed above it.
    fn placement(&mut self) -> LinePlacement {
        LinePlacement {
            band: self.current(),
            offset_top: std::mem::take(&mut self.pending_offset),
        }
    }

    fn current(&self) -> FloatBand {
        self.bands[self.index.min(self.bands.len() - 1)]
    }

    /// Line boxes of `line_height` still free in the current band. The last band is open-ended,
    /// so everything left goes there.
    fn lines_left(&self, line_height: f32) -> usize {
        let band = self.current();
        let Some(height) = band.height else {
            return usize::MAX;
        };
        if line_height <= 0.0 {
            return usize::MAX;
        }
        (((height - self.used) / line_height).floor().max(0.0)) as usize
    }

    /// Charge `lines` line boxes of `line_height` to the current band, moving on when it fills.
    fn take_lines(&mut self, lines: usize, line_height: f32) {
        let Some(height) = self.current().height else {
            return;
        };
        self.used += lines as f32 * line_height.max(0.0);
        if self.used >= height {
            self.advance();
        }
    }

    /// Move to the next band. Returns false when this was already the last one.
    fn advance(&mut self) -> bool {
        if self.index + 1 >= self.bands.len() {
            return false;
        }
        if let Some(height) = self.current().height {
            self.pending_offset += (height - self.used).max(0.0);
        }
        self.index += 1;
        self.used = 0.0;
        true
    }

    /// Give up on band-by-band placement: everything else goes in the final, open-ended band.
    fn exhaust(&mut self) {
        while self.advance() {}
    }
}

const DEFAULT_FONT_FAMILY: &str = "sans-serif";

/// Parse an HTML presentational length attribute (e.g. `<img width="80">`) into pixels.
/// Accepts a bare integer/float or a trailing `px`; ignores `%` and other units.
fn parse_px_attr(v: &str) -> Option<f32> {
    let s = v.trim();
    let s = s.strip_suffix("px").unwrap_or(s);
    s.trim().parse::<f32>().ok().filter(|n| *n >= 0.0)
}

// Cache key: (text, font_family, size_bits, line_height_bits, weight, max_width_bits,
// letter_spacing_bits). Floats are stored as their bit pattern so the tuple is Hash + Eq.
type MeasureKey = (String, String, u32, u32, i32, u32, u32);

/// CSS `text-align` on a block, as `justify_content` for the anonymous flex containers holding its
/// line boxes. A line box *is* that container, so this is what positions a run too short to fill it
/// - a run that wraps already fills the line and is aligned by the shaper instead.
///
/// `justify` stays `None`: the shaper stretches a wrapped run itself, and flexing a single item
/// can't emulate that.
fn line_box_justify(align: &Value) -> Option<taffy::JustifyContent> {
    let Value::TextAlign(ta) = align else {
        return None;
    };
    match ta {
        TextAlign::Center => Some(taffy::JustifyContent::CENTER),
        TextAlign::End | TextAlign::Right => Some(taffy::JustifyContent::FLEX_END),
        _ => None,
    }
}

/// One entry in a run of inline content awaiting layout. `Item`s are normal inline boxes/text
/// laid out inside an anonymous flex container; `Break` is a `<br>` that ends the current line box
/// and, when standing alone, contributes an empty line of the carried line-height.
enum InlineEntry {
    Item(LayoutElementId, TaffyNodeId),
    Break(f64),
}

/// A [`TaffyTree`] that can move between threads.
///
/// # Why this is needed
///
/// The engine keeps a layouter across frames so a resize can re-run taffy over the tree it
/// already built instead of constructing a new one - about half of layout time. That layouter
/// lives on the tab's `BrowsingContext`, whose worker is a `tokio::spawn`ed task, and such a
/// task may resume on a different thread after each await. So everything it holds must be
/// `Send`. `TaffyTree` is not.
///
/// # Safety
///
/// `TaffyTree` is `!Send` because `CompactLength` packs its value into a tagged word declared
/// `*const ()`, and Rust treats any raw pointer as non-thread-safe. That word only ever holds a
/// real address for `calc()` values, which taffy stores as an *opaque handle* it never
/// dereferences - its own docs say the value "may be a pointer, index, etc." and it is only
/// ever handed back to a caller-supplied resolver. Taffy cannot promise `Send` because it
/// cannot know what its callers put there; taffy's maintainer accordingly recommends exactly
/// this wrapper, sound "so long as you are either not using `Calc`, or your calc type
/// implements `Send + Sync`" (DioxusLabs/taffy#949, and see #823).
///
/// We are in the first case, and enforce it with the compiler rather than by convention: this
/// crate builds taffy **without the `calc` feature** (see `Cargo.toml`), so
/// `CompactLength::calc` does not exist and no code path can put an address in that word. Every
/// `CompactLength` we construct is a tag plus an `f32`. There is therefore no pointer to
/// invalidate by moving the tree, and no interior sharing: the wrapper is `Send` but
/// deliberately **not** `Sync`, matching how it is used - owned by one tab, moved with its
/// task, never shared between threads.
///
/// If taffy's `calc` feature is ever switched back on, this impl must be re-justified or
/// removed. Upstream's proper fix is DioxusLabs/taffy#855 (generic over the calc type), a draft
/// since 2025; when it lands this wrapper can go.
struct SendTaffyTree(TaffyTree<TaffyContext>);

// SAFETY: see the type's doc comment. Holds no pointer while taffy's `calc` feature is off,
// which this crate enforces in Cargo.toml.
#[allow(unsafe_code)]
unsafe impl Send for SendTaffyTree {}

/// The whole point of [`SendTaffyTree`]: the engine retains a `TaffyLayouter` on a tab worker
/// whose future must be `Send`. Asserted at compile time, in every build, because losing it
/// would silently drop the engine back to rebuilding the layout tree on every resize.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    let _ = assert_send::<TaffyLayouter>;
};

impl std::ops::Deref for SendTaffyTree {
    type Target = TaffyTree<TaffyContext>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for SendTaffyTree {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Layouter structure that uses taffy as layout engine
pub struct TaffyLayouter {
    tree: SendTaffyTree,
    root_id: TaffyNodeId,
    layout_taffy_mapping: HashMap<LayoutElementId, TaffyNodeId>,
    /// Maps each layout element that lives inside an anonymous flex container to that
    /// container's taffy node id. The anonymous container exists in the taffy tree (between
    /// the real parent and its inline children) but has no corresponding LayoutElementNode.
    /// populate_boxmodel uses this to add the container's taffy-computed offset to the offset
    /// it passes down to the child, which would otherwise be missing from the calculation.
    anon_container_map: HashMap<LayoutElementId, TaffyNodeId>,
    /// Media store for loading images/SVGs during layout. Shared (Arc) so the media loaded
    /// here is visible to the rasterization stage, which looks resources up by the same id.
    media_store: Arc<MediaStore>,
    /// Locked once per measurement call rather than across the whole layout pass, so other
    /// threads (e.g. the rasterizer) can access the font collection between calls. `dyn` so the
    /// same instance can be shared with the rasterizer and swapped for a non-Parley impl.
    font_system: Arc<Mutex<dyn FontSystem>>,
    /// Taffy calls the measure function 2-4× per node (MinContent, MaxContent, actual width);
    /// memoizing eliminates the redundant Parley shaping calls.
    measure_cache: HashMap<MeasureKey, Size<f32>>,
    /// Reverse index used by the table post-processing pass.
    dom_to_layout_mapping: HashMap<DomNodeId, LayoutElementId>,
    /// Per-block `(left inset, line width)` line-box geometry that clears the floats beside that
    /// block, in CSS pixels. Empty on the first layout pass, since a float's position is not known
    /// until that pass has run; filled in from its result for the second.
    float_insets: HashMap<DomNodeId, Vec<FloatBand>>,
    /// Width each `display: table` box settled on in the previous pass, pinned onto the box when
    /// the tree is rebuilt. A table's width comes out of the column algorithm, which runs after
    /// taffy - so on the first pass taffy laid the contents out at the wrong width, and anything
    /// that wraps (most visibly a caption) wrapped to it. Replaying the width lets that content
    /// be measured at the width it will actually have.
    table_widths: HashMap<DomNodeId, f32>,
    /// Taffy insets for absolutely positioned boxes that stretch between opposing insets,
    /// rebased from their CSS containing block onto the parent taffy measures from. Empty on the
    /// first pass - a box's containing block is only known once the page has been laid out - and
    /// replayed on the second so taffy sizes those boxes itself. See [`RebasedInsets`].
    abspos_insets: HashMap<DomNodeId, RebasedInsets>,
}

/// Apply the CSS `text-transform` keyword to a text run. `uppercase`/`lowercase` map the whole
/// string; `capitalize` uppercases the first letter of each whitespace-separated word. `none`
/// (and any unsupported keyword such as `full-width`) leaves the text unchanged.
fn apply_text_transform(text: String, transform: Value) -> String {
    let Value::Keyword(id) = transform else {
        return text;
    };
    match lookup(id).as_str() {
        "uppercase" => text.cow_to_uppercase().into_owned(),
        "lowercase" => text.cow_to_lowercase().into_owned(),
        "capitalize" => {
            let mut out = String::with_capacity(text.len());
            let mut at_word_start = true;
            for ch in text.chars() {
                if ch.is_whitespace() {
                    at_word_start = true;
                    out.push(ch);
                } else if at_word_start {
                    at_word_start = false;
                    out.extend(ch.to_uppercase());
                } else {
                    out.push(ch);
                }
            }
            out
        }
        _ => text,
    }
}

/// Context structures to pass to taffy measure functions so we can calculate the size of the text or images.
#[derive(Clone, Debug)]
pub enum TaffyContext {
    Text(ElementContextText),
    Image(ElementContextImage),
    Svg(ElementContextSvg),
}

impl TaffyContext {
    fn text(
        text: &str,
        font_info: FontInfo,
        node_id: DomNodeId,
        text_offset: Coordinate,
        no_wrap: bool,
    ) -> TaffyContext {
        TaffyContext::Text(ElementContextText {
            node_id,
            font_info,
            text: text.to_string(),
            text_offset,
            no_wrap,
            available_width: 0.0,
        })
    }

    fn image(
        src: &str,
        media_id: MediaId,
        dimension: geo::Dimension,
        node_id: DomNodeId,
        placeholder: bool,
        alt: Option<String>,
    ) -> TaffyContext {
        TaffyContext::Image(ElementContextImage {
            node_id,
            src: src.to_string(),
            media_id,
            dimension,
            placeholder,
            alt,
        })
    }

    fn svg(src: &str, media_id: MediaId, dimension: geo::Dimension, node_id: DomNodeId) -> TaffyContext {
        TaffyContext::Svg(ElementContextSvg {
            node_id,
            src: src.to_string(),
            media_id,
            dimension,
        })
    }
}

impl Default for TaffyLayouter {
    fn default() -> Self {
        Self::new()
    }
}

impl TaffyLayouter {
    /// Create a layouter with its own font system.
    ///
    /// To share the font collection with other components (e.g. a `VelloRasterizer`)
    /// use [`TaffyLayouter::with_font_system`] and pass the same `Arc` to both.
    pub fn new() -> Self {
        Self::with_font_system(Arc::new(Mutex::new(ParleyFontSystem::new())))
    }

    /// Create a layouter that shares an existing font system.
    pub fn with_font_system(font_system: Arc<Mutex<dyn FontSystem>>) -> Self {
        Self {
            tree: SendTaffyTree(TaffyTree::new()),
            root_id: TaffyNodeId::new(0),
            layout_taffy_mapping: HashMap::new(),
            anon_container_map: HashMap::new(),
            media_store: Arc::new(MediaStore::new()),
            font_system,
            measure_cache: HashMap::new(),
            dom_to_layout_mapping: HashMap::new(),
            float_insets: HashMap::new(),
            table_widths: HashMap::new(),
            abspos_insets: HashMap::new(),
        }
    }

    /// Expose the font system so callers can share it with other components.
    pub fn font_system(&self) -> Arc<Mutex<dyn FontSystem>> {
        Arc::clone(&self.font_system)
    }

    /// Share an external media store with this layouter. Resources loaded during layout are
    /// stored here; passing the same store to the rasterizer lets it resolve those resources
    /// by id. Without this they live in two separate stores and images render as placeholders.
    pub fn set_media_store(&mut self, media_store: Arc<MediaStore>) {
        self.media_store = media_store;
    }

    /// The media store shared by this layouter (see [`set_media_store`](Self::set_media_store)).
    pub fn media_store(&self) -> Arc<MediaStore> {
        Arc::clone(&self.media_store)
    }

    pub fn print_tree(&mut self) {
        self.tree.print_tree(self.root_id);
    }
}

impl CanLayout for TaffyLayouter {
    fn layout(
        &mut self,
        render_tree: RenderTree,
        viewport: Option<geo::Dimension>,
        // DPI scaling is applied later in the pipeline; text is measured in CSS pixels.
        _dpi_scale_factor: f32,
    ) -> LayoutTree {
        let Some(root_id) = render_tree.root_id else {
            log::error!("Render tree has no root node; was parse() called? Returning empty layout.");
            return LayoutTree {
                render_tree,
                arena: HashMap::new(),
                root_id: LayoutElementId::new(0),
                next_node_id: Arc::new(RwLock::new(LayoutElementId::new(0))),
                root_dimension: geo::Dimension::ZERO,
            };
        };

        // Two things are only knowable once the page has been laid out once: where a float landed
        // (text has to be wrapped around it) and which containing block an absolutely positioned
        // box actually belongs to (a box stretching between opposing insets has to be sized
        // against it). Both are collected on the first pass and replayed on a second. A page that
        // needs neither - most of them - pays for one pass.
        self.float_insets.clear();
        self.abspos_insets.clear();
        self.table_widths.clear();
        let (mut layout_tree, placed, stretched, table_widths) = self.layout_pass(render_tree, root_id, viewport);

        // Float bands are resolved in document order from this one baseline layout rather than by
        // laying the page out over and over until the answer stops moving - which it did not: see
        // `resolve_bands_in_document_order`. Two passes total, always.
        let insets = resolve_bands_in_document_order(&layout_tree, &placed);
        if insets.is_empty() && stretched.is_empty() && table_widths.is_empty() {
            dump_layout_to_json(&layout_tree);
            return layout_tree;
        }

        self.float_insets = insets;
        self.abspos_insets = stretched;
        self.table_widths = table_widths;
        let (final_tree, _, _, _) = self.layout_pass(layout_tree.render_tree, root_id, viewport);
        layout_tree = final_tree;
        self.float_insets.clear();
        self.abspos_insets.clear();
        self.table_widths.clear();
        dump_layout_to_json(&layout_tree);
        layout_tree
    }
}

/// Writes the settled box geometry of every element to the JSON file named by `GOSUB_DUMP_LAYOUT`,
/// the layout counterpart of `GOSUB_DUMP_CSS`. Off unless the variable is set.
///
/// Each entry carries the element's tag, `id`/`class`, tree depth and border box, in document
/// order - enough to see which boxes ended up on top of each other, which is not recoverable
/// from a screenshot.
fn dump_layout_to_json(layout_tree: &LayoutTree) {
    let Ok(path) = std::env::var("GOSUB_DUMP_LAYOUT") else {
        return;
    };

    let doc = &layout_tree.render_tree.doc;
    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut stack = vec![(layout_tree.root_id, 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(el) = layout_tree.arena.get(&id) else {
            continue;
        };
        // Children are pushed in reverse so the walk emits them in document order.
        stack.extend(el.children.iter().rev().map(|child| (*child, depth + 1)));

        // Text boxes are included as `#text` with their content: a box that is the wrong size
        // because its *text* went missing is invisible in an element-only dump.
        let (tag, id_attr, class_attr) = match doc.get_node_by_id(el.dom_node_id) {
            Some(Node {
                node_type: NodeType::Element(element),
                ..
            }) => (
                element.tag_name.clone(),
                element.attributes.get("id").cloned().unwrap_or_default(),
                element.attributes.get("class").cloned().unwrap_or_default(),
            ),
            _ => match &el.context {
                ElementContext::Text(text) => ("#text".to_string(), String::new(), text.text.clone()),
                _ => continue,
            },
        };

        let b = el.box_model.border_box;
        entries.push(serde_json::json!({
            "depth": depth,
            "node_id": u64::from(el.dom_node_id),
            "tag": tag,
            "id": id_attr,
            "class": class_attr,
            "x": b.x,
            "y": b.y,
            "w": b.width,
            "h": b.height,
        }));
    }

    match serde_json::to_string_pretty(&entries) {
        Ok(json) => match std::fs::write(&path, json) {
            Ok(()) => log::info!("Layout dump written to {path} ({} elements)", entries.len()),
            Err(e) => log::error!("Failed to write layout dump to {path}: {e}"),
        },
        Err(e) => log::error!("Failed to serialize layout dump: {e}"),
    }
}

impl TaffyLayouter {
    /// One full layout: build the taffy tree, compute it, convert to box models and place floats.
    /// Returns the tree and the floats that were placed.
    fn layout_pass(
        &mut self,
        render_tree: RenderTree,
        root_id: RenderNodeId,
        viewport: Option<geo::Dimension>,
    ) -> (
        LayoutTree,
        Vec<crate::layouter::float::PlacedFloat>,
        HashMap<DomNodeId, RebasedInsets>,
        HashMap<DomNodeId, f32>,
    ) {
        let mut layout_tree = self.generate_tree(render_tree, root_id);

        let (placed, stretched, table_widths) = self.compute_and_populate(&mut layout_tree, viewport);
        (layout_tree, placed, stretched, table_widths)
    }
}

impl TaffyLayouter {
    /// Recompute geometry on the taffy tree that is already built, without touching its
    /// structure or any node's style.
    ///
    /// Valid only when nothing that feeds tree construction has changed - no restyle, no new
    /// intrinsic sizes - which in practice means a viewport resize. Everything taffy needs is
    /// already in the tree, so this skips the ~half of layout that goes into building it.
    /// See `BrowsingContext`'s `DamageLevel::Geometry`.
    /// Floats and absolutely positioned boxes are re-placed against the new geometry, but the
    /// two-pass feedback is discarded: this path deliberately does not rebuild the taffy tree, so
    /// the rebased insets and float line-boxes baked into it on the last full layout are reused.
    /// A change that invalidates those needs a full rebuild, which is what `DamageLevel::Layout`
    /// and above ask for.
    pub fn relayout(&mut self, layout_tree: &mut LayoutTree, viewport: Option<geo::Dimension>) {
        let _ = self.compute_and_populate(layout_tree, viewport);
    }

    /// Run taffy over the current tree and write the results back as box models, then run the
    /// post-passes that need settled positions: tables, floats, absolute positioning. The half of
    /// `layout` that does not depend on how the tree was built.
    ///
    /// Returns what a second pass would need - the floats that were placed, and the absolutely
    /// positioned boxes that have to be re-sized against their real containing block.
    fn compute_and_populate(
        &mut self,
        layout_tree: &mut LayoutTree,
        viewport: Option<geo::Dimension>,
    ) -> (
        Vec<crate::layouter::float::PlacedFloat>,
        HashMap<DomNodeId, RebasedInsets>,
        HashMap<DomNodeId, f32>,
    ) {
        // // Compute the layout based on the viewport
        let size = match viewport {
            Some(viewport) => Size {
                width: AvailableSpace::Definite(viewport.width as f32),
                height: AvailableSpace::Definite(viewport.height as f32),
            },
            None => Size::MAX_CONTENT,
        };

        // Clone the Arc and take the measure cache so the closure can capture them
        // without holding a borrow of `self` while `self.tree` is mutably borrowed.
        let font_system = Arc::clone(&self.font_system);
        let mut measure_cache: HashMap<MeasureKey, Size<f32>> = std::mem::take(&mut self.measure_cache);

        if let Err(e) = self
            .tree
            .compute_layout_with_measure(self.root_id, size, |v_kd, v_as, _v_ni, v_nc, _v_s| {
                // If taffy already knows both dimensions, no measurement needed.
                if let (Some(w), Some(h)) = (v_kd.width, v_kd.height) {
                    return Size { width: w, height: h };
                }

                match v_nc {
                    Some(TaffyContext::Text(text_ctx)) => {
                        let max_width = if text_ctx.no_wrap {
                            // white-space: nowrap - measure at unlimited width so text never wraps
                            1_000_000_000.0_f64
                        } else {
                            match v_as.width {
                                AvailableSpace::Definite(width) => width as f64,
                                AvailableSpace::MaxContent => 1_000_000_000.0, // f64::MAX doesn't work. Seems some kind of overflow. Same goes for f32::MAX
                                AvailableSpace::MinContent => 0.0,
                            }
                        };

                        let cache_key: MeasureKey = (
                            text_ctx.text.clone(),
                            text_ctx.font_info.family.clone(),
                            (text_ctx.font_info.size as f32).to_bits(),
                            (text_ctx.font_info.line_height as f32).to_bits(),
                            text_ctx.font_info.weight,
                            (max_width as f32).to_bits(),
                            (text_ctx.font_info.letter_spacing as f32).to_bits(),
                        );
                        if let Some(&cached) = measure_cache.get(&cache_key) {
                            return cached;
                        }

                        // Measure through the shared font system. The lock is released
                        // immediately after the call so other callers (e.g. the
                        // rasterizer) can interleave without contention.
                        let text_layout = {
                            let mut fs = font_system.lock();
                            get_text_layout(text_ctx.text.as_str(), &text_ctx.font_info, max_width, &mut *fs)
                        };
                        match text_layout {
                            Ok(text_layout) => {
                                // Ceil width to the nearest CSS pixel. Parley returns a fractional
                                // f64 width; when taffy truncates to f32 and feeds that back as
                                // available_width, parley re-measures with slightly less space than
                                // the text requires and wraps. Ceiling ensures allocated width ≥
                                // natural text width, preventing spurious wrapping at the boundary.
                                let mut width = text_layout.width.ceil() as f32;

                                // Parley strips trailing whitespace (including NBSP) from the line-box
                                // advance width. When we appended U+00A0 as a trailing-space marker
                                // for a text node that ended with whitespace, that NBSP is never
                                // counted by parley, so taffy under-allocates and pango clips it.
                                // Detect the marker and add the missing space width manually.
                                // Whitespace-only nodes ("\u{00A0}") have their width fixed explicitly
                                // in the taffy style, so the measure callback is not invoked for them.
                                if text_ctx.text.ends_with('\u{00A0}') && text_ctx.text != "\u{00A0}" {
                                    width += (text_ctx.font_info.size * 0.3) as f32;
                                }

                                let result = Size {
                                    width,
                                    // Ceil height so the layout height matches the integer-pixel surface
                                    // that pango creates (prevents descenders from overflowing the box).
                                    height: text_layout.height.ceil() as f32,
                                };
                                measure_cache.insert(cache_key, result);
                                result
                            }
                            Err(_) => Size::ZERO,
                        }
                    }
                    // Replaced elements: honour whichever dimension CSS has constrained and
                    // derive the other from the intrinsic aspect ratio, so e.g. an
                    // `height: 30px` logo keeps its shape instead of stretching to its full
                    // intrinsic width.
                    Some(TaffyContext::Image(image_ctx)) => measure_replaced(v_kd, image_ctx.dimension),
                    // SVG-backed <img> elements carry their intrinsic size the same way.
                    // Without this arm they measured as 0×0 and collapsed (e.g. the HN logo).
                    Some(TaffyContext::Svg(svg_ctx)) => measure_replaced(v_kd, svg_ctx.dimension),
                    _ => Size::ZERO,
                }
            })
        {
            log::error!("Failed to compute taffy layout: {:?}", e);
            self.measure_cache = measure_cache;
            return (Vec::new(), HashMap::new(), HashMap::new());
        }
        self.measure_cache = measure_cache;

        // Since we are not interested in taffy layout after this stage in the pipeline, we convert
        // the taffy layout to a box model layout tree. This makes the rest of the pipeline
        // layout-engine agnostic.
        let root_id = layout_tree.root_id;
        let root_width = layout_tree.root_dimension.width;
        self.populate_boxmodel(layout_tree, root_id, Coordinate::ZERO, root_width);
        let table_widths = post_process_tables(layout_tree, &self.dom_to_layout_mapping);
        // After tables: a float inside a table cell must be placed against the cell's final
        // position, which lattice only fixes during the table pass.
        let placed = post_process_floats(layout_tree);

        // Publish the root's settled size *before* the absolute-positioning pass. That pass falls
        // back to it for the initial containing block when no viewport is given, and
        // `root_dimension` is still `ZERO` from `generate_tree` until this runs - so percentage
        // insets resolved against zero and `right`/`bottom` placement came out at negative
        // offsets. The root is not absolutely positioned, so `post_process_abspos` cannot change
        // its box; moving this up is safe.
        if let Some(root) = layout_tree.get_node_by_id(root_id) {
            let w = root.box_model.margin_box.width as f32;
            let h = root.box_model.margin_box.height as f32;
            layout_tree.root_dimension = geo::Dimension::new(w as f64, h as f64);
        }

        // Last: an absolutely positioned box is measured from its containing block's *final*
        // position, so every ancestor - tables and floats included - must have settled first.
        let icb = viewport.unwrap_or(layout_tree.root_dimension);
        let stretched = post_process_abspos(layout_tree, icb);

        (placed, stretched, table_widths)
    }

    fn populate_boxmodel(
        &self,
        layout_tree: &mut LayoutTree,
        layout_node_id: LayoutElementId,
        offset: Coordinate,
        parent_content_width: f64,
    ) {
        let Some(taffy_node_id) = self.layout_taffy_mapping.get(&layout_node_id) else {
            log::warn!("No taffy mapping for layout node {:?}", layout_node_id);
            return;
        };
        let Ok(layout) = self.tree.layout(*taffy_node_id) else {
            log::warn!("Failed to get taffy layout for node {:?}", taffy_node_id);
            return;
        };
        let layout = *layout;

        // The anonymous flex container wrapping this node *is* its line box.
        let line_box_width = self
            .anon_container_map
            .get(&layout_node_id)
            .and_then(|anon| self.tree.layout(*anon).ok())
            .map(|l| l.size.width as f64);

        let Some(el) = layout_tree.get_node_by_id_mut(layout_node_id) else {
            log::warn!("Layout node {:?} not found in arena", layout_node_id);
            return;
        };
        el.box_model = taffy_layout_to_boxmodel(&layout, offset);
        // For text nodes, available_width is the wrap limit passed to the renderer, so it has to
        // be the width of the *line box*, not of the block. The two differ when a float shortens
        // the line boxes: shaping at the block's width there would lay the text out in one long
        // run straight through the float. Fall back to the block's content width for text that is
        // not inside an anonymous line container.
        if let ElementContext::Text(ref mut text_ctx) = el.context {
            text_ctx.available_width = line_box_width.unwrap_or(parent_content_width);
        }
        let my_content_width = el.box_model.content_box.width;
        let child_ids = el.children.clone();

        // Inline elements (those placed in an anonymous flex container by their parent) do not
        // establish a new containing block. Their children should inherit the enclosing block's
        // content width so that Skia uses the same wrap boundary that Parley used during layout.
        // Without this, Skia receives the inline element's shrunk natural width and wraps text
        // that Parley measured as a single line, causing height mismatches and overlapping content.
        let is_inline_node = self.anon_container_map.contains_key(&layout_node_id);
        let content_width_for_children = if is_inline_node {
            parent_content_width
        } else {
            my_content_width
        };

        // Absolute position of this node's content area - used as the base offset for direct children.
        let children_offset = Coordinate::new(offset.x + layout.location.x as f64, offset.y + layout.location.y as f64);

        for child_id in child_ids {
            // If this child lives inside an anonymous flex container (created by process_inlines),
            // its taffy position is relative to that container, not to the current node. Add the
            // anonymous container's own taffy-computed offset so the absolute position is correct.
            let anon_offset = if let Some(&anon_taffy_id) = self.anon_container_map.get(&child_id) {
                if let Ok(anon_layout) = self.tree.layout(anon_taffy_id) {
                    Coordinate::new(anon_layout.location.x as f64, anon_layout.location.y as f64)
                } else {
                    Coordinate::ZERO
                }
            } else {
                Coordinate::ZERO
            };

            self.populate_boxmodel(
                layout_tree,
                child_id,
                Coordinate::new(children_offset.x + anon_offset.x, children_offset.y + anon_offset.y),
                content_width_for_children,
            );
        }
    }

    fn generate_tree(&mut self, render_tree: RenderTree, root_id: RenderNodeId) -> LayoutTree {
        self.measure_cache.clear();
        self.tree = SendTaffyTree(TaffyTree::new());
        // Taffy's built-in rounding snaps layout values to integer CSS pixels, which causes
        // text containers to lose sub-pixel width (e.g. 52.344 → 52.0). This makes pango
        // render at a surface too narrow for the text and produces spurious line wraps.
        // Our renderer handles DPR scaling itself via ceil(width) * dpr, so we disable
        // taffy's rounding here.
        self.tree.disable_rounding();
        self.root_id = TaffyNodeId::new(0); // Will be filled in later
        self.layout_taffy_mapping.clear();
        self.anon_container_map.clear();
        self.dom_to_layout_mapping.clear();

        let mut layout_tree = LayoutTree {
            render_tree,
            arena: HashMap::new(),
            root_id: LayoutElementId::new(0), // Will be filled in later
            next_node_id: Arc::new(RwLock::new(LayoutElementId::new(0))),
            root_dimension: geo::Dimension::ZERO,
        };

        let Some((layout_element_root_id, taffy_root_id)) = self.generate_taffy_element(&mut layout_tree, root_id)
        else {
            log::error!("Failed to generate taffy element for root node {:?}", root_id);
            return layout_tree;
        };

        layout_tree.root_id = layout_element_root_id;
        self.root_id = taffy_root_id;

        layout_tree
    }

    // Process inline elements by adding them to the taffy tree, wrapped in anonymous flex
    // containers. A run with no `<br>` produces a single wrapping container (the old behaviour); a
    // run containing `<br>` is split into one container per line box, which the block parent stacks
    // vertically - that is how a `<br>` becomes a line break.
    fn process_inlines(
        &mut self,
        layout_tree: &LayoutTree,
        current_inline_group: &[InlineEntry],
        element_node: &mut LayoutElementNode,
        leaf_id: TaffyNodeId,
        justify: Option<taffy::JustifyContent>,
    ) {
        log::debug!("Processing inline elements: {:?}", current_inline_group.len());

        if current_inline_group.is_empty() {
            return;
        }

        // Line boxes beside a float are shorter than the ones below it, and the anonymous
        // container *is* the line box here, so a block crossed by a float needs one container per
        // band rather than one for the whole block. `cursor` carries the position in that band
        // list across the whole block, including over `<br>` boundaries.
        // A table cell is a flex column, so the axis that moves a line box across the cell is the
        // cross axis - the cell's `align_items`, set from its `text-align`. An `align_self` on the
        // line box would override that, so inside a cell it is left off. Wikipedia's infobox
        // section headers are `text-align: center` and came out flush left for want of this.
        let line_style = LineStyle {
            justify,
            cell_aligned: matches!(
                layout_tree
                    .render_tree
                    .doc
                    .get_style(element_node.dom_node_id, &StyleProperty::Display),
                Value::Display(CssDisplay::TableCell)
            ),
        };
        let bands = self.float_insets.get(&element_node.dom_node_id).cloned();
        let mut cursor = bands.as_ref().map(|bands| BandCursor::new(bands));

        // Split the run into line boxes at `<br>` boundaries. An empty segment (consecutive `<br>`s
        // or a leading `<br>`) still emits a line box of the break's line-height, so runs of `<br>`
        // produce blank lines rather than collapsing.
        let mut segment: Vec<(LayoutElementId, TaffyNodeId)> = Vec::new();
        for entry in current_inline_group {
            match entry {
                InlineEntry::Item(id, taffy) => segment.push((*id, *taffy)),
                InlineEntry::Break(lh) => {
                    if segment.is_empty() {
                        let placement = cursor.as_mut().map(BandCursor::placement);
                        if let Some(c) = cursor.as_mut() {
                            c.take_lines(1, *lh as f32);
                        }
                        self.emit_line(&[], Some(*lh), element_node, leaf_id, line_style, placement);
                    } else {
                        self.emit_banded(
                            layout_tree,
                            &segment,
                            element_node,
                            leaf_id,
                            line_style,
                            cursor.as_mut(),
                        );
                        segment.clear();
                        // The break itself ends the line the segment left open.
                        if let Some(c) = cursor.as_mut() {
                            c.take_lines(1, *lh as f32);
                        }
                    }
                }
            }
        }
        if !segment.is_empty() {
            self.emit_banded(
                layout_tree,
                &segment,
                element_node,
                leaf_id,
                line_style,
                cursor.as_mut(),
            );
        }
    }

    /// Emit one run of inline items, split across the float bands it flows through.
    ///
    /// Without bands this is a single container, exactly as before. With them, the items are
    /// packed into lines at each band's width until that band is full, and what is left starts a
    /// new container in the next band - which is how text runs narrow beside a float and then
    /// returns to full width underneath it.
    fn emit_banded(
        &mut self,
        layout_tree: &LayoutTree,
        items: &[(LayoutElementId, TaffyNodeId)],
        element_node: &mut LayoutElementNode,
        leaf_id: TaffyNodeId,
        line_style: LineStyle,
        cursor: Option<&mut BandCursor>,
    ) {
        let Some(cursor) = cursor else {
            self.emit_line(items, None, element_node, leaf_id, line_style, None);
            return;
        };

        let line_height = self.inline_line_height(layout_tree, items);
        let mut rest = items;
        while !rest.is_empty() {
            let band = cursor.current();
            // Measuring every item lets the split match where taffy will actually wrap. If any
            // item cannot be measured the split would be guesswork, so the whole run goes into
            // one container at the current band's width instead.
            let capacity = cursor.lines_left(line_height);
            let Some((taken, lines)) = self.fill_band(layout_tree, rest, band, capacity) else {
                let placement = cursor.placement();
                cursor.exhaust();
                self.emit_line(rest, None, element_node, leaf_id, line_style, Some(placement));
                return;
            };
            if taken == 0 {
                // Nothing fits this band - a float leaves it too narrow for even one item - so
                // drop to the next one rather than emitting an empty container. CSS puts a line
                // that cannot fit beside a float below it, which is exactly this.
                if !cursor.advance() {
                    self.emit_line(rest, None, element_node, leaf_id, line_style, Some(cursor.placement()));
                    return;
                }
                continue;
            }
            // A space that falls at a band boundary is a line break's worth of whitespace, which
            // CSS collapses away. Keeping it also risked a blank line: a trailing space pushed
            // past the band's width wraps to a line of its own in that container.
            let chunk_end = trim_trailing_whitespace(layout_tree, &rest[..taken]);
            let placement = cursor.placement();
            self.emit_line(
                &rest[..chunk_end],
                None,
                element_node,
                leaf_id,
                line_style,
                Some(placement),
            );
            cursor.take_lines(lines, line_height);
            rest = skip_leading_whitespace(layout_tree, &rest[taken..]);
        }
    }

    /// The line height to charge each line of `items` against a band's height. Taken from the
    /// first text item, which is what decides the line box's height in practice.
    fn inline_line_height(&self, layout_tree: &LayoutTree, items: &[(LayoutElementId, TaffyNodeId)]) -> f32 {
        for (id, _) in items {
            if let Some(el) = layout_tree.arena.get(id) {
                if let ElementContext::Text(text) = &el.context {
                    if text.font_info.line_height > 0.0 {
                        return text.font_info.line_height as f32;
                    }
                }
            }
        }
        DEFAULT_FONT_SIZE as f32
    }

    /// How many leading items of `rest` fit in `band` within `max_lines` line boxes, and how many
    /// lines they take. `None` when an item's width cannot be measured.
    fn fill_band(
        &mut self,
        layout_tree: &LayoutTree,
        rest: &[(LayoutElementId, TaffyNodeId)],
        band: FloatBand,
        max_lines: usize,
    ) -> Option<(usize, usize)> {
        if max_lines == 0 {
            return Some((0, 0));
        }
        let width = band.line_width as f64;
        let mut lines = 1usize;
        let mut used = 0.0_f64;
        for (i, (id, _)) in rest.iter().enumerate() {
            let item_width = self.inline_item_width(layout_tree, id)?;
            // The first item on a line always goes on it, however wide: a word longer than the
            // line overflows rather than vanishing, which is what taffy does too.
            if used > 0.0 && used + item_width > width {
                if lines == max_lines {
                    return Some((i, lines));
                }
                lines += 1;
                used = 0.0;
            }
            used += item_width;
        }
        Some((rest.len(), lines))
    }

    /// Natural width of one inline item, measured the same way taffy will measure it.
    fn inline_item_width(&mut self, layout_tree: &LayoutTree, id: &LayoutElementId) -> Option<f64> {
        match &layout_tree.arena.get(id)?.context {
            ElementContext::Text(text) => {
                let font_info = text.font_info.clone();
                let content = text.text.clone();
                let mut font_system = self.font_system.lock();
                get_text_layout(&content, &font_info, MAX_CONTENT_WIDTH, &mut *font_system)
                    .ok()
                    .map(|d| d.width)
            }
            ElementContext::Image(image) => Some(image.dimension.width),
            ElementContext::Svg(svg) => Some(svg.dimension.width),
            ElementContext::None => {
                // A float or an absolutely positioned box is in the inline run only because that
                // is where it was written; it is out of flow and takes no room on the line. The
                // float that *causes* the bands is usually the first item in the very run being
                // split, so counting it would be wrong twice over.
                let dom_id = layout_tree.arena.get(id)?.dom_node_id;
                let doc = &layout_tree.render_tree.doc;
                if position_is_out_of_flow(&**doc, dom_id) || float_side(&**doc, dom_id).is_some() {
                    return Some(0.0);
                }
                // Anything else with no context - an inline-block, say - has no measurable width
                // until taffy runs, so the caller falls back rather than guessing.
                None
            }
        }
    }

    /// Emit one line box as an anonymous flex container holding `items`. When `items` is empty and
    /// `empty_line_height` is `Some`, the container is pinned to that height so a blank line (from a
    /// standalone `<br>`) keeps its vertical extent; an empty line with no height is skipped.
    fn emit_line(
        &mut self,
        items: &[(LayoutElementId, TaffyNodeId)],
        empty_line_height: Option<f64>,
        element_node: &mut LayoutElementNode,
        leaf_id: TaffyNodeId,
        line_style: LineStyle,
        placement: Option<LinePlacement>,
    ) {
        // All inline elements (even a single one) are wrapped in an anonymous flex container.
        // This ensures the text measure function always receives AvailableSpace::Definite from
        // the flex algorithm, preventing single-child text nodes from getting MaxContent width
        // (which would make them lay out on one line and overflow their block parent).
        let mut style = Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            // The block's `text-align`: positions runs that don't fill the line box.
            justify_content: line_style.justify,
            align_self: (!line_style.cell_aligned).then_some(AlignSelf::FLEX_START),
            // FlexStart ensures multi-row intrinsic height = sum of all row heights.
            // Taffy's default (None = Stretch) fails to include wrapped rows in the
            // container's auto height, causing rows beyond the first to overflow.
            align_content: Some(AlignContent::FLEX_START),
            gap: Size {
                width: LengthPercentage::length(0.0),
                height: LengthPercentage::length(0.0),
            },
            size: Size {
                width: Dimension::auto(),
                height: Dimension::auto(),
            },
            ..Default::default()
        };
        // Line boxes - not the block itself - are what a float shortens, and the anonymous
        // container *is* the line box here, so the inset goes on its margins. The block keeps its
        // full width, so its background and borders still span the float, as CSS requires.
        if let Some(placement) = placement {
            style.margin.left = LengthPercentageAuto::length(placement.band.left_inset);
            style.size.width = Dimension::from_length(placement.band.line_width);
            if placement.offset_top > 0.0 {
                style.margin.top = LengthPercentageAuto::length(placement.offset_top);
            }
        }
        if items.is_empty() {
            match empty_line_height {
                // No child can give the line height, so pin it to the break's line-height.
                Some(lh) => style.size.height = Dimension::from_length(lh as f32),
                None => return,
            }
        }

        let Ok(taffy_container_id) = self.tree.new_leaf(style) else {
            return;
        };
        if let Err(e) = self.tree.add_child(leaf_id, taffy_container_id) {
            log::warn!("Failed to add anonymous container to taffy tree: {:?}", e);
        }

        for (inline_layout_element_id, inline_taffy_node_id) in items {
            if let Err(e) = self.tree.add_child(taffy_container_id, *inline_taffy_node_id) {
                log::warn!("Failed to add inline child to taffy tree: {:?}", e);
            }
            element_node.children.push(*inline_layout_element_id);
            // Record that this layout element sits inside an anonymous container so that
            // populate_boxmodel can add the container's taffy-computed offset.
            self.anon_container_map
                .insert(*inline_layout_element_id, taffy_container_id);
        }
    }

    /// Split a text node in a *mixed* inline run (alongside inline-level elements) into one inline
    /// box per word, so text wraps around its sibling inline boxes like a browser line box - an
    /// atomic per-node text box can only wrap as a whole, jumping to its own line instead.
    ///
    /// Words are separated by explicit single-space boxes; each space is its own flex item so it
    /// doubles as a wrap point and carries exactly one space's width - attaching it to the word
    /// would double-count against the trailing-NBSP fudge in the measure callback.
    fn push_text_words(
        &mut self,
        layout_tree: &mut LayoutTree,
        text_node: &Node,
        render_node_id: RenderNodeId,
        group: &mut Vec<InlineEntry>,
    ) {
        let (had_leading, had_trailing, words) = {
            let NodeType::Text(full) = &text_node.node_type else {
                return;
            };
            (
                full.starts_with(|c: char| c.is_ascii_whitespace()),
                full.ends_with(|c: char| c.is_ascii_whitespace()),
                split_collapsible_whitespace(full)
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
            )
        };
        if words.is_empty() {
            return;
        }

        // Interleave words with single-space tokens: [ ]? w0 [ ] w1 [ ] … [ ]?
        let mut tokens: Vec<String> = Vec::with_capacity(words.len() * 2 + 1);
        if had_leading {
            tokens.push(" ".to_string());
        }
        let last = words.len() - 1;
        for (i, word) in words.into_iter().enumerate() {
            tokens.push(word);
            if i < last {
                tokens.push(" ".to_string());
            }
        }
        if had_trailing {
            tokens.push(" ".to_string());
        }

        for tok in tokens {
            let mut token_node = text_node.clone();
            token_node.node_type = NodeType::Text(tok);
            if let Some(pair) = self.build_text_word_leaf(layout_tree, &token_node, render_node_id) {
                group.push(InlineEntry::Item(pair.0, pair.1));
            }
        }
    }

    /// Build a single-word inline text box, reusing `extract_taffy_data` so font/whitespace
    /// resolution matches the whole-node path. `dom_to_layout_mapping` is intentionally not written
    /// - one text node maps to many word boxes and the single-slot map cannot represent that.
    fn build_text_word_leaf(
        &mut self,
        layout_tree: &mut LayoutTree,
        word_node: &Node,
        render_node_id: RenderNodeId,
    ) -> Option<(LayoutElementId, TaffyNodeId)> {
        let (taffy_context, taffy_style) = self.extract_taffy_data(layout_tree, word_node)?;
        let element_context = to_element_context(taffy_context.as_ref());
        let taffy_id = match taffy_context {
            Some(ctx) => self.tree.new_leaf_with_context(taffy_style, ctx).ok()?,
            None => self.tree.new_leaf(taffy_style).ok()?,
        };
        let element_node = LayoutElementNode {
            id: layout_tree.next_node_id(),
            dom_node_id: word_node.node_id,
            render_node_id,
            parent: None,
            box_model: box_model::BoxModel::ZERO,
            children: vec![],
            context: element_context,
            background_media: None,
        };
        let layout_element_id = element_node.id;
        layout_tree.arena.insert(layout_element_id, element_node);
        self.layout_taffy_mapping.insert(layout_element_id, taffy_id);
        Some((layout_element_id, taffy_id))
    }

    // Process node and turn it into a taffy node. It will recursively process any children and takes care to wrap any multiple inline elements
    // into an anonymous taffy block element. This way we can sort of emulate inline elements within taffy.
    fn generate_taffy_element(
        &mut self,
        layout_tree: &mut LayoutTree,
        render_node_id: RenderNodeId,
    ) -> Option<(LayoutElementId, TaffyNodeId)> {
        let render_node = layout_tree.render_tree.get_node_by_id(render_node_id)?;
        let dom_node = layout_tree
            .render_tree
            .doc
            .get_node_by_id(DomNodeId::from(render_node.node_id))?;

        let (taffy_context, taffy_style) = self.extract_taffy_data(layout_tree, &dom_node)?;

        // `text-align` inherits, so this is the block's computed value; the line boxes below are
        // anonymous and have no style of their own to read.
        let line_justify = line_box_justify(
            &layout_tree
                .render_tree
                .doc
                .get_style(dom_node.node_id, &StyleProperty::TextAlign),
        );

        // Flex and grid containers are formatting contexts where ALL children - inline or block -
        // are direct layout participants. Wrapping inline children in an anonymous flex container
        // would insert an extra level that breaks the parent's `gap`, `align-items`, etc.
        // Flex and grid containers are formatting contexts where ALL children - inline or block -
        // are direct layout participants. Wrapping inline children in an anonymous flex container
        // would insert an extra level that breaks the parent's `gap`, `align-items`, etc.
        //
        // The table displays are mapped onto taffy flex containers too, but a cell is a block
        // container and its inline content still belongs in line boxes - without that, a cell that
        // stacks its children puts every word on a line of its own. Only the table boxes are
        // excluded, rather than asking the CSS display outright: *inline* elements are mapped onto
        // flex containers as well - 12000 of them on this page - and must keep what they have.
        // An inline box does not start a line - it continues its parent's - so whitespace at its
        // start is only leading whitespace if the line itself is empty, which this element cannot
        // know. Dropping it regardless lost the space in
        // `1964<span>;</span><span> </span>62 years ago`, which rendered as "1964;62 years ago":
        // Parsoid gives every entity its own element, so a single space routinely arrives as the
        // whole content of one. A block container *does* start a line, and there the trim is right.
        let starts_a_line = !matches!(
            layout_tree
                .render_tree
                .doc
                .get_style(dom_node.node_id, &StyleProperty::Display),
            Value::Display(CssDisplay::Inline)
        );

        let parent_is_flex_or_grid = matches!(taffy_style.display, Display::Flex | Display::Grid)
            && !matches!(
                layout_tree
                    .render_tree
                    .doc
                    .get_style(dom_node.node_id, &StyleProperty::Display),
                Value::Display(
                    CssDisplay::Table
                        | CssDisplay::TableCaption
                        | CssDisplay::TableCell
                        | CssDisplay::TableFooterGroup
                        | CssDisplay::TableHeaderGroup
                        | CssDisplay::TableRow
                        | CssDisplay::TableRowGroup
                )
            );

        // The context will be moved to the taffy tree, so we need to convert it before that happens.
        let element_context = match taffy_context {
            Some(ref ctx) => to_element_context(Some(ctx)),
            None => to_element_context(None),
        };

        let result = match taffy_context {
            Some(ctx) => self.tree.new_leaf_with_context(taffy_style.to_owned(), ctx),
            None => self.tree.new_leaf(taffy_style.to_owned()),
        };

        let Ok(leaf_id) = result else {
            return None;
        };

        let background_media = self.resolve_background_media(layout_tree, dom_node.node_id);

        let mut element_node = LayoutElementNode {
            id: layout_tree.next_node_id(),
            dom_node_id: dom_node.node_id,
            render_node_id,
            // Back-patched below once all children are attached (covers block + inline children).
            parent: None,
            box_model: box_model::BoxModel::ZERO,
            children: vec![],
            context: element_context,
            background_media,
        };

        // Children are tracked in both the taffy tree and the element_node's children vec.
        let mut current_inline_group = Vec::new();
        // Track how many trailing whitespace-only text nodes are at the end of the current inline
        // group so they can be stripped before flushing, mirroring how leading whitespace is dropped.
        // Trailing whitespace (e.g. "\n" after the last text node inside a <p>) would otherwise
        // produce an empty flex row in the anonymous container, adding a spurious blank line.
        let mut trailing_ws_count = 0usize;
        // An inline `<svg>` is a replaced element: usvg has already parsed the whole subtree and
        // the painter draws the graphic from that tree, so laying the children out again would
        // both duplicate them and stop taffy seeing the `<svg>` as a leaf - and a non-leaf never
        // has its measure function called, which is the only thing that gives the element its
        // intrinsic size. Without this the graphic collapses to nothing unless CSS sizes it.
        let render_node_children = if matches!(element_node.context, ElementContext::Svg(_)) {
            Vec::new()
        } else {
            render_node.children.clone()
        };

        // A "mixed" inline run - a (non-flex/grid) element with at least one inline-level *element*
        // child, not just text - needs its text nodes split into per-word boxes so text flows and
        // wraps around the inline boxes like a browser line box. Pure-text blocks (no inline-element
        // children) keep the single whole-node run to preserve Parley text shaping/justification.
        let has_inline_element_child = !parent_is_flex_or_grid
            && render_node_children.iter().any(|cid| {
                layout_tree
                    .render_tree
                    .get_document_node_by_render_id(*cid)
                    .is_some_and(|n| {
                        matches!(n.node_type, NodeType::Element(_))
                            && (n.is_inline_element() || n.is_inline_block_element())
                    })
            });

        for child_id in render_node_children.iter() {
            let Some(child_node) = layout_tree.render_tree.get_document_node_by_render_id(*child_id) else {
                continue;
            };

            // In a mixed inline run, split text into per-word inline boxes (see push_text_words).
            // Whitespace-only nodes fall through to the normal NBSP-separator path below.
            if has_inline_element_child {
                if let NodeType::Text(text) = &child_node.node_type {
                    if !is_collapsible_whitespace(text) {
                        self.push_text_words(layout_tree, &child_node, *child_id, &mut current_inline_group);
                        trailing_ws_count = 0;
                        continue;
                    }
                }
            }

            let Some((child_layout_element_id, child_taffy_id)) = self.generate_taffy_element(layout_tree, *child_id)
            else {
                continue;
            };

            // In a flex/grid parent every child is a direct layout participant - inline or block -
            // so skip the anonymous-container wrapping and add them straight to the parent.
            if parent_is_flex_or_grid {
                // Still discard pure-whitespace text nodes; they carry no visual content.
                if let NodeType::Text(text) = &child_node.node_type {
                    if is_collapsible_whitespace(text) {
                        // Drop leading whitespace (before any inline sibling) of a box that
                        // starts a line. Keep inter-element whitespace - it collapses to a single
                        // space in extract_taffy_data and visually separates adjacent inline
                        // elements (e.g. between </span><span>).
                        if current_inline_group.is_empty() && starts_a_line {
                            continue;
                        }
                    }
                }
                if let Err(e) = self.tree.add_child(leaf_id, child_taffy_id) {
                    log::warn!("Failed to add child to taffy tree: {:?}", e);
                }
                element_node.children.push(child_layout_element_id);
                continue;
            }

            // Don't add inline elements to the taffy tree yet. We need to group them first and possibly wrap inside a block
            if child_node.is_inline_element() || child_node.is_inline_block_element() || child_node.is_text() {
                // <br> is a forced line break, not a paintable inline item. Record a break marker
                // carrying the line-height (for the case it stands alone as an empty line) and skip
                // adding its taffy node as a flex item; process_inlines splits the run here.
                if matches!(&child_node.node_type, NodeType::Element(d) if d.tag_name.eq_ignore_ascii_case("br")) {
                    let doc = &layout_tree.render_tree.doc;
                    let nid = child_node.node_id;
                    let font_size = match doc.get_style(nid, &StyleProperty::FontSize) {
                        Value::Unit(v, Unit::Px) => v as f64,
                        _ => DEFAULT_FONT_SIZE,
                    };
                    let line_height = match doc.get_style(nid, &StyleProperty::LineHeight) {
                        Value::Unit(v, Unit::Px) => v as f64,
                        Value::Number(ratio) => font_size * ratio as f64,
                        _ => font_size * 1.4,
                    };
                    current_inline_group.push(InlineEntry::Break(line_height));
                    trailing_ws_count = 0;
                    continue;
                }
                let is_ws = if let NodeType::Text(text) = &child_node.node_type {
                    // ASCII, not `str::trim`: `trim` uses the Unicode whitespace set, so a text
                    // node holding only an `&nbsp;` looked like source formatting and was skipped
                    // as leading whitespace. Parsoid gives every entity its own element, which is
                    // how Wikipedia's `Designed<span>&nbsp;</span>by` lost its space entirely.
                    if is_collapsible_whitespace(text) {
                        // Drop leading whitespace (before any inline sibling) of a box that
                        // starts a line. Keep inter-element whitespace - it collapses to a single
                        // space in extract_taffy_data and visually separates adjacent inline
                        // elements (e.g. between </span><span>).
                        if current_inline_group.is_empty() && starts_a_line {
                            continue;
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                log::debug!("Pushing element as inline: {:?}", child_node.node_id);
                current_inline_group.push(InlineEntry::Item(child_layout_element_id, child_taffy_id));
                if is_ws {
                    trailing_ws_count += 1;
                } else {
                    trailing_ws_count = 0;
                }
                continue;
            }

            log::debug!("Element {:?} is not an inline", child_node.node_id);

            // Strip trailing whitespace before flushing, then flush.
            current_inline_group.truncate(current_inline_group.len().saturating_sub(trailing_ws_count));
            self.process_inlines(
                layout_tree,
                &current_inline_group,
                &mut element_node,
                leaf_id,
                line_justify,
            );
            current_inline_group = Vec::new();
            trailing_ws_count = 0;

            if let Err(e) = self.tree.add_child(leaf_id, child_taffy_id) {
                log::warn!("Failed to add child to taffy tree: {:?}", e);
            }
            element_node.children.push(child_layout_element_id);
        }

        // Strip trailing whitespace and deal with any remaining inline elements
        current_inline_group.truncate(current_inline_group.len().saturating_sub(trailing_ws_count));
        self.process_inlines(
            layout_tree,
            &current_inline_group,
            &mut element_node,
            leaf_id,
            line_justify,
        );

        // The layout-tree is the structure handed to the rest of the pipeline; taffy stays
        // internal to this layouter so other layout engines can be swapped in.
        let layout_element_id = element_node.id;
        let child_ids = element_node.children.clone();
        layout_tree.arena.insert(layout_element_id, element_node);
        // Point every child (block and inline) back at this node so the containing block can be
        // found by walking up - e.g. the cage for `position: sticky`.
        for child_id in child_ids {
            if let Some(child) = layout_tree.arena.get_mut(&child_id) {
                child.parent = Some(layout_element_id);
            }
        }

        // Create a mapping between the layout element id and the taffy node id. We need this to generate
        // the boxmodel at a later time in this pipeline stage.
        self.layout_taffy_mapping.insert(layout_element_id, leaf_id);
        self.dom_to_layout_mapping.insert(dom_node.node_id, layout_element_id);

        Some((layout_element_id, leaf_id))
    }

    /// Resolves the element's CSS `background-image` (if any) to a media id: reads the computed
    /// value, resolves the URL against the document base URL, and loads it into the media store.
    /// Returns `None` when there is no background image or it fails to load.
    fn resolve_background_media(&self, layout_tree: &LayoutTree, dom_node_id: DomNodeId) -> Option<BackgroundMedia> {
        let doc = &layout_tree.render_tree.doc;
        let url = match doc.get_style(dom_node_id, &StyleProperty::BackgroundImage) {
            Value::Keyword(id) => lookup(id),
            _ => return None,
        };
        if url.is_empty() || url.eq_ignore_ascii_case("none") {
            return None;
        }

        let abs = to_absolute_url(&url, &doc.base_url());
        // Non-blocking: while the background image is still fetching, render without it; the reflow
        // after the fetch completes paints it in.
        let media_id = match self.media_store.request_media(&abs) {
            MediaRequest::Ready(media_id) => media_id,
            MediaRequest::Pending => return None,
        };

        let layout = doc.background_image_layout(dom_node_id);

        // Store the intrinsic size + layout; the painter finalizes the tile geometry once the box
        // is known. A raster image is used directly. An SVG background is rasterized to a raster
        // tile at a box-independent size (its intrinsic size, or an explicit `background-size`
        // length) so it reuses the single raster path for repeat / cover / contain; `compute_bg_tiling`
        // then scales that raster for cover/contain once the box is known. (An SVG intrinsic size is
        // typically large - e.g. 400×300 - so cover/contain downscale and stay crisp.)
        match &*self.media_store.get(media_id, MediaType::Image) {
            Media::Image(mi) => Some(BackgroundMedia::Image {
                media_id,
                natural: (mi.image.width() as f32, mi.image.height() as f32),
                layout,
            }),
            Media::Svg(ms) => {
                let size = ms.svg.tree.size();
                let (rw, rh) = match layout.size {
                    BgSize::Length(w, h) => (w, h),
                    _ => (size.width(), size.height()),
                };
                let rw = (rw.round() as u32).max(1);
                let rh = (rh.round() as u32).max(1);
                match self.media_store.svg_raster_tile(media_id, rw, rh) {
                    Some(raster_id) => Some(BackgroundMedia::Image {
                        media_id: raster_id,
                        natural: (rw as f32, rh as f32),
                        layout,
                    }),
                    // Rasterization failed - fall back to the (stretch) SVG paint path.
                    None => Some(BackgroundMedia::Svg(media_id)),
                }
            }
        }
    }

    /// Extracts taffy variables based the DOM node. It will generate the taffy style based on the node CSS properties,
    /// any context that might be needed (images, svg, text).
    fn extract_taffy_data(&self, layout_tree: &LayoutTree, dom_node: &Node) -> Option<(Option<TaffyContext>, Style)> {
        let mut taffy_context = None;
        let mut taffy_style = Style::default();

        match &dom_node.node_type {
            NodeType::Element(data) => {
                let conv = CssTaffyConverter::new(dom_node.node_id, &*layout_tree.render_tree.doc);
                taffy_style = conv.convert(false);

                // Second pass only: pin the width the column algorithm settled on, so the
                // contents are measured at the width the table will actually have. A caption is
                // the visible case - it is laid out across the finished table, so on the first
                // pass (where taffy sizes the box from its own contents) it wraps to the wrong
                // width and then drags the table out to match.
                if let Some(&width) = self.table_widths.get(&dom_node.node_id) {
                    taffy_style.size.width = Dimension::from_length(width);
                    // A cell is a flex item with `flex_grow: 1`, which would stretch it back to
                    // an equal share of its row and undo the pin. It only shows up in a row that
                    // does not span every column - the dialect table's second header row holds
                    // two cells under a `colspan=2` title, and they grew to half the table each,
                    // leaving their centred text far to the right of the cells lattice then
                    // moved into place.
                    taffy_style.flex_grow = 0.0;
                    taffy_style.flex_shrink = 0.0;
                }

                // Second pass only: replace the CSS insets of an absolutely positioned box that
                // stretches between opposing insets with ones rebased onto its parent, so taffy
                // sizes it against the CSS containing block rather than whatever ancestor happens
                // to be its parent. Empty on the first pass. See `abspos::RebasedInsets`.
                if let Some(rebased) = self.abspos_insets.get(&dom_node.node_id) {
                    if let (Some(l), Some(r)) = (rebased.left, rebased.right) {
                        taffy_style.inset.left = LengthPercentageAuto::length(l);
                        taffy_style.inset.right = LengthPercentageAuto::length(r);
                    }
                    if let (Some(t), Some(b)) = (rebased.top, rebased.bottom) {
                        taffy_style.inset.top = LengthPercentageAuto::length(t);
                        taffy_style.inset.bottom = LengthPercentageAuto::length(b);
                    }
                }

                // Images get a taffy context so their intrinsic size participates in layout.
                if data.tag_name.eq_ignore_ascii_case("img") {
                    let base_url = layout_tree.render_tree.doc.base_url();
                    let Some(src) = data.get_attribute("src") else {
                        log::warn!("img element missing src attribute");
                        return None;
                    };
                    let src = to_absolute_url(src, &base_url);

                    log::debug!("Loading (image) resource: {}", src);

                    // Non-blocking: an uncached image kicks off a background fetch and returns
                    // Pending without stalling layout. The element is kept with a placeholder size
                    // (HTML width/height attrs if present, else 0×0); a reflow lands once the fetch
                    // completes and installs the real intrinsic size.
                    match self.media_store.request_media(src.as_str()) {
                        MediaRequest::Ready(media_id) => {
                            let media = self.media_store.get(media_id, MediaType::Image);
                            // When the media is a placeholder (load failed), use a small fixed
                            // size so the broken-image icon doesn't blow up the layout. The
                            // rasterizer scales the icon to whatever rect the element actually
                            // occupies, so display quality is unaffected.
                            let is_placeholder = self.media_store.is_placeholder(media_id);
                            // Resolve the intrinsic size, whether this is an SVG, and whether the
                            // decoded raster is fully transparent (nothing visible to paint) - all
                            // in one borrow.
                            let (dimension, is_svg, is_transparent) = match media.borrow() {
                                // Use the SVG's intrinsic size so the element gets a non-zero box.
                                // A failed/placeholder load uses the same small fixed size as images.
                                Media::Svg(media_svg) => {
                                    let d = if is_placeholder {
                                        geo::Dimension::new(32.0, 32.0)
                                    } else {
                                        let size = media_svg.svg.tree.size();
                                        geo::Dimension::new(size.width() as f64, size.height() as f64)
                                    };
                                    (d, true, false)
                                }
                                Media::Image(media_image) => {
                                    let d = if is_placeholder {
                                        geo::Dimension::new(32.0, 32.0)
                                    } else {
                                        geo::Dimension::new(
                                            media_image.image.width() as f64,
                                            media_image.image.height() as f64,
                                        )
                                    };
                                    // `.all()` short-circuits on the first opaque pixel, so this is
                                    // cheap for the common (visible) image and only scans fully when
                                    // the image really is transparent.
                                    let transparent = !is_placeholder
                                        && media_image.image.width() > 0
                                        && media_image
                                            .image
                                            .as_raw()
                                            .as_chunks::<4>()
                                            .0
                                            .iter()
                                            .all(|px| px[3] == 0);
                                    (d, false, transparent)
                                }
                            };

                            // Pin the intrinsic aspect ratio so a block-level replaced element keeps
                            // its shape when only one axis is constrained (e.g. `width:100%; height:auto`
                            // inside a `figure`). Without this, taffy's block layout leaves height
                            // unconstrained and the image stretches to fill its box. Skip it when the
                            // author fixed BOTH axes to definite lengths - the explicit box wins then,
                            // matching CSS `aspect-ratio: auto` for replaced elements.
                            if !is_placeholder && dimension.width > 0.0 && dimension.height > 0.0 {
                                let both_fixed = taffy_style.size.width.into_option().is_some()
                                    && taffy_style.size.height.into_option().is_some();
                                let ratio = (dimension.width / dimension.height) as f32;
                                if !both_fixed && ratio.is_finite() && ratio > 0.0 {
                                    taffy_style.aspect_ratio = Some(ratio);
                                }
                            }

                            // A broken/placeholder image still reserves the author's declared
                            // box (HTML width/height attrs), matching Firefox, which draws a small
                            // broken-image icon inside that reserved space rather than collapsing.
                            if is_placeholder {
                                if let Some(w) = data.get_attribute("width").and_then(|s| parse_px_attr(s)) {
                                    taffy_style.size.width = Dimension::from_length(w);
                                }
                                if let Some(h) = data.get_attribute("height").and_then(|s| parse_px_attr(s)) {
                                    taffy_style.size.height = Dimension::from_length(h);
                                }
                            }

                            // Browsers show the `alt` text only when the image itself renders
                            // nothing useful: a broken/placeholder load, or a fully transparent
                            // image. A normally-decoded, visible image never shows its alt.
                            let alt = if is_placeholder || is_transparent {
                                data.get_attribute("alt")
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                            } else {
                                None
                            };

                            taffy_context = Some(if is_svg {
                                TaffyContext::svg(src.as_str(), media_id, dimension, dom_node.node_id)
                            } else {
                                TaffyContext::image(
                                    src.as_str(),
                                    media_id,
                                    dimension,
                                    dom_node.node_id,
                                    is_placeholder,
                                    alt,
                                )
                            });
                        }
                        MediaRequest::Pending => {
                            // Placeholder size: honour the HTML width/height attributes if present,
                            // otherwise leave whatever CSS sizing convert() produced (0×0 for a bare
                            // <img>). The reflow after the fetch completes installs the real size.
                            if let Some(w) = data.get_attribute("width").and_then(|s| parse_px_attr(s)) {
                                taffy_style.size.width = Dimension::from_length(w);
                            }
                            if let Some(h) = data.get_attribute("height").and_then(|s| parse_px_attr(s)) {
                                taffy_style.size.height = Dimension::from_length(h);
                            }
                        }
                    }
                }

                if data.tag_name.eq_ignore_ascii_case("svg") {
                    let inner_html = layout_tree.render_tree.doc.inner_html(dom_node.node_id);
                    match self
                        .media_store
                        .load_media_from_data(MediaType::Svg, inner_html.into_bytes().as_slice())
                    {
                        Ok(media_id) => {
                            let media = self.media_store.get(media_id, MediaType::Svg);
                            let dimension = match media.borrow() {
                                Media::Svg(media_svg) => {
                                    let size = media_svg.svg.tree.size();
                                    geo::Dimension::new(size.width() as f64, size.height() as f64)
                                }
                                _ => geo::Dimension::ZERO,
                            };
                            taffy_context = Some(TaffyContext::svg(
                                "gosub://internal",
                                media_id,
                                dimension,
                                dom_node.node_id,
                            ));
                        }
                        Err(e) => {
                            log::warn!("Could not load SVG media: {:?}", e);
                        }
                    }
                }
            }
            NodeType::Text(text) => {
                let parent_node = match dom_node.parent_id {
                    Some(parent_id) => layout_tree.render_tree.doc.get_node_by_id(parent_id),
                    None => None,
                };
                parent_node.as_ref()?;

                let doc = &layout_tree.render_tree.doc;

                let mut font_size = DEFAULT_FONT_SIZE;
                let mut font_family = DEFAULT_FONT_FAMILY.to_string();

                if let Value::Unit(value, Unit::Px) = doc.get_style(dom_node.node_id, &StyleProperty::FontSize) {
                    font_size = value as f64;
                }

                if let Value::Keyword(id) = doc.get_style(dom_node.node_id, &StyleProperty::FontFamily) {
                    font_family = lookup(id);
                }

                let font_weight = match doc.get_style(dom_node.node_id, &StyleProperty::FontWeight) {
                    Value::FontWeight(weight) => match weight {
                        FontWeight::Normal => 400.0,
                        FontWeight::Bold => 700.0,
                        FontWeight::Number(value) => value as f64,
                        FontWeight::Bolder => 700.0,
                        FontWeight::Lighter => 300.0,
                    },
                    _ => 400.0,
                };

                let font_italic = matches!(
                    doc.get_style(dom_node.node_id, &StyleProperty::FontStyle),
                    Value::Keyword(id) if lookup(id) == "italic"
                );

                // `left`/`right` are physical and `start`/`end` logical; they only coincide in LTR,
                // which is all the pipeline handles today. Collapse them here rather than in the
                // cascade, so the distinction survives for when direction is honoured.
                let alignment = match doc.get_style(dom_node.node_id, &StyleProperty::TextAlign) {
                    Value::TextAlign(value) => match value {
                        TextAlign::Center => FontAlignment::Center,
                        TextAlign::End | TextAlign::Right => FontAlignment::End,
                        TextAlign::Justify => FontAlignment::Justify,
                        _ => FontAlignment::Start,
                    },
                    _ => FontAlignment::Start,
                };

                let line_height = match doc.get_style(dom_node.node_id, &StyleProperty::LineHeight) {
                    Value::Unit(value, Unit::Px) => value as f64,
                    Value::Number(ratio) => font_size * ratio as f64,
                    // CSS "normal" line-height. We use 1.4 instead of the CSS-spec minimum of
                    // ~1.2 because pango and parley use different font metrics tables. Parley
                    // (layout) may return a smaller height than pango (raster), so without this
                    // buffer the rendered text surface can exceed the span's background
                    // rectangle, making descenders (e.g. "p") appear to overflow the colored box.
                    _ => font_size * 1.4,
                };

                // Calculate vertical offset for centering based on the line height.
                let text_offset = Coordinate::new(0.0, (line_height - font_size) / 2.0);

                // Apply CSS white-space: normal - collapse newlines/runs of whitespace to a
                // single space and strip leading/trailing whitespace.  Raw HTML text nodes
                // contain the literal source indentation (e.g. "\n    Red box…\n  ") which
                // pango would render as a blank first line if left untouched.
                // Whitespace-only source nodes (e.g. "\n  " between </span><span>) collapse
                // to a single space so they produce an inter-element gap when kept.
                // Two different "all whitespace" questions. *Collapsible* whitespace (spaces,
                // tabs, newlines) is source formatting that collapses to one space. Whitespace
                // that CSS does not collapse - U+00A0 and the other fixed-width spaces - is
                // content, and the node must keep exactly what it says.
                let collapsible_only = !text.is_empty() && text.chars().all(|c: char| c.is_ascii_whitespace());
                let whitespace_only = !text.is_empty() && text.chars().all(char::is_whitespace);
                // Preserve one leading/trailing inter-element gap as NBSP (non-breaking) so
                // pango does not wrap at the boundary space, while still rendering a visible gap.
                let had_leading_space = text.starts_with(|c: char| c.is_ascii_whitespace());
                let had_trailing_space = text.ends_with(|c: char| c.is_ascii_whitespace());
                let mut text: String = split_collapsible_whitespace(text).collect::<Vec<_>>().join(" ");
                if !collapsible_only {
                    if had_leading_space && !text.is_empty() {
                        text.insert(0, '\u{00A0}');
                    }
                    if had_trailing_space && !text.is_empty() {
                        text.push('\u{00A0}');
                    }
                }
                if collapsible_only {
                    // Inter-element whitespace (e.g. between </span><span>). Collapse to a single
                    // NBSP so the text context is non-empty and pango does not wrap at it.
                    text = "\u{00A0}".to_string();
                }
                if whitespace_only {
                    // Parley measures a run of only whitespace as 0 wide when called with
                    // MinContent (max_advance=0), which collapses the flex item to nothing, so the
                    // width is set explicitly and `flex_shrink` pinned. This used to apply only to
                    // collapsible whitespace, so a node holding just an `&nbsp;` came out 0 wide -
                    // and since Parsoid gives every entity its own element, Wikipedia's
                    // `Designed<span>&nbsp;</span>by` rendered as "Designedby".
                    // A provisional width; the real one is measured below, once the font is
                    // known. This stands in if that measurement comes back empty.
                    let space_width = (font_size * 0.3) as f32;
                    taffy_style.size.width = Dimension::from_length(space_width);
                    taffy_style.flex_shrink = 0.0;
                }
                // if inline_element_counter > 0 {
                //     // If we are in an inline container, we need to add a space between the text nodes
                //     text = format!(" {}", text).clone()
                // }

                let no_wrap = matches!(
                    doc.get_style(dom_node.node_id, &StyleProperty::WhiteSpace),
                    Value::Keyword(id) if lookup(id) == "nowrap"
                );
                if no_wrap {
                    taffy_style.flex_shrink = 0.0;
                }

                // Apply `text-transform` (inherited from the parent element) to the run before it
                // is measured and painted - TaffyContext::text is the single source used for both,
                // so transforming here keeps layout width and drawn glyphs in sync.
                let text = apply_text_transform(text, doc.get_style(dom_node.node_id, &StyleProperty::TextTransform));

                let text_decoration = match doc.get_style(dom_node.node_id, &StyleProperty::TextDecorationLine) {
                    Value::Keyword(id) => lookup(id),
                    _ => String::new(),
                };

                // `letter-spacing` arrives already resolved to px (em resolved against font-size in
                // `get_style`); `normal` (a keyword) means no extra spacing.
                let letter_spacing = match doc.get_style(dom_node.node_id, &StyleProperty::LetterSpacing) {
                    Value::Unit(px, Unit::Px) => px as f64,
                    _ => 0.0,
                };

                let font_info = FontInfo {
                    family: font_family,
                    size: font_size,
                    weight: font_weight as i32,
                    width: 100, // 100%, normal
                    slant: if font_italic { 1 } else { 0 },
                    line_height,
                    letter_spacing,
                    alignment,
                    underline: text_decoration.contains("underline"),
                    line_through: text_decoration.contains("line-through"),
                };

                // The gap between two words of a mixed inline run is one of these whitespace
                // boxes, so a width a fifth too wide reads as loose spacing across a whole
                // paragraph. Measure it rather than estimating: the text is a non-breaking space
                // by now, which - unlike a plain one - parley will not trim away, so its advance
                // at max-content is the font's real space width.
                if whitespace_only {
                    let measured = {
                        let mut font_system = self.font_system.lock();
                        get_text_layout(&text, &font_info, MAX_CONTENT_WIDTH, &mut *font_system)
                            .ok()
                            .map(|d| d.width as f32)
                            .filter(|w| *w > 0.0)
                    };
                    if let Some(width) = measured {
                        taffy_style.size.width = Dimension::from_length(width);
                    }
                }

                taffy_context = Some(TaffyContext::text(
                    text.as_str(),
                    font_info,
                    dom_node.node_id,
                    text_offset,
                    no_wrap,
                ));
            }
            NodeType::Comment(_) => {
                // No need to layout for comment nodes. In fact, they should have been removed already
                // by the render-tree building stage.
                return None;
            }
        }

        Some((taffy_context, taffy_style))
    }
}

// Convert a URI to an absolute URL based on the base URL if this is needed
fn to_absolute_url(uri: &str, base_uri: &str) -> String {
    // Already-absolute references (http(s)://, file://, data:, blob:, …) are returned as-is.
    if let Ok(parsed) = url::Url::parse(uri) {
        return parsed.to_string();
    }

    // Otherwise resolve the relative reference against the document base URL using proper URL
    // join semantics: this replaces the base's last path segment (so `assets/x.png` against
    // `http://h/page.html` becomes `http://h/assets/x.png`, not `.../page.html/assets/x.png`),
    // handles leading-slash absolute paths and protocol-relative `//host/...` references, and
    // collapses `.`/`..`.
    match url::Url::parse(base_uri).and_then(|base| base.join(uri)) {
        Ok(joined) => joined.to_string(),
        // Base URL unusable (e.g. empty for an inline document) - fall back to the raw reference.
        Err(_) => uri.to_string(),
    }
}

/// Measure a replaced element (image / SVG) honouring any dimension CSS has already
/// constrained. When only one of width/height is known, the other is derived from the
/// intrinsic aspect ratio so the element keeps its shape; when neither is known the
/// intrinsic size is used as-is. (The both-known case is short-circuited before the
/// measure callback reaches this point, but is handled here for completeness.)
fn measure_replaced(known: Size<Option<f32>>, intrinsic: geo::Dimension) -> Size<f32> {
    let iw = intrinsic.width as f32;
    let ih = intrinsic.height as f32;
    match (known.width, known.height) {
        (Some(w), Some(h)) => Size { width: w, height: h },
        (Some(w), None) => Size {
            width: w,
            height: if iw > 0.0 { w * ih / iw } else { ih },
        },
        (None, Some(h)) => Size {
            width: if ih > 0.0 { h * iw / ih } else { iw },
            height: h,
        },
        (None, None) => Size { width: iw, height: ih },
    }
}

/// Convert a taffy context to an element context. Optionally, these two structures should be merged
/// and only ElementContext should be used.
fn to_element_context(taffy_context: Option<&TaffyContext>) -> ElementContext {
    match taffy_context {
        Some(TaffyContext::Text(text_ctx)) => ElementContext::text(
            text_ctx.text.as_str(),
            text_ctx.font_info.clone(),
            text_ctx.node_id,
            text_ctx.text_offset,
            text_ctx.no_wrap,
        ),
        Some(TaffyContext::Image(image_ctx)) => ElementContext::image(
            image_ctx.src.as_str(),
            image_ctx.media_id,
            image_ctx.dimension,
            image_ctx.node_id,
            image_ctx.placeholder,
            image_ctx.alt.clone(),
        ),
        Some(TaffyContext::Svg(svg_ctx)) => ElementContext::svg(
            svg_ctx.src.as_str(),
            svg_ctx.media_id,
            svg_ctx.dimension,
            svg_ctx.node_id,
        ),
        None => ElementContext::None,
    }
}

/// Converts a taffy layout to our own BoxModel structure
pub fn taffy_layout_to_boxmodel(layout: &Layout, offset: Coordinate) -> box_model::BoxModel {
    box_model::BoxModel::new(
        // Border box
        geo::Rect::new(
            offset.x + layout.location.x as f64,
            offset.y + layout.location.y as f64,
            layout.size.width as f64,
            layout.size.height as f64,
        ),
        Edges {
            top: layout.padding.top as f64,
            right: layout.padding.right as f64,
            bottom: layout.padding.bottom as f64,
            left: layout.padding.left as f64,
        },
        Edges {
            top: layout.border.top as f64,
            right: layout.border.right as f64,
            bottom: layout.border.bottom as f64,
            left: layout.border.left as f64,
        },
        Edges {
            top: layout.margin.top as f64,
            right: layout.margin.right as f64,
            bottom: layout.margin.bottom as f64,
            left: layout.margin.left as f64,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{apply_text_transform, to_absolute_url};
    use crate::common::document::style::{intern, Value};

    fn kw(s: &str) -> Value {
        Value::Keyword(intern(s))
    }

    #[test]
    fn text_transform_uppercase_lowercase() {
        assert_eq!(apply_text_transform("Working".to_string(), kw("uppercase")), "WORKING");
        assert_eq!(apply_text_transform("Working".to_string(), kw("lowercase")), "working");
    }

    #[test]
    fn text_transform_capitalize() {
        assert_eq!(
            apply_text_transform("early stage".to_string(), kw("capitalize")),
            "Early Stage"
        );
    }

    #[test]
    fn text_transform_none_and_unsupported_passthrough() {
        assert_eq!(apply_text_transform("Working".to_string(), kw("none")), "Working");
        // Unsupported keyword (e.g. full-width) leaves the text untouched.
        assert_eq!(apply_text_transform("Working".to_string(), kw("full-width")), "Working");
        // Non-keyword value passes through.
        assert_eq!(
            apply_text_transform("Working".to_string(), Value::Number(1.0)),
            "Working"
        );
    }

    #[test]
    fn relative_ref_replaces_base_last_segment() {
        // A relative reference resolves against the document, *replacing* the page file -
        // not appended after it (the bug this guards against).
        assert_eq!(
            to_absolute_url("assets/photo.jpg", "http://localhost:8765/image-test.html"),
            "http://localhost:8765/assets/photo.jpg"
        );
        assert_eq!(
            to_absolute_url("../up.png", "http://h/a/b/page.html"),
            "http://h/a/up.png"
        );
    }

    #[test]
    fn root_relative_and_protocol_relative() {
        assert_eq!(
            to_absolute_url("/img/y.png", "http://localhost:8765/deep/page.html"),
            "http://localhost:8765/img/y.png"
        );
        assert_eq!(
            to_absolute_url("//cdn.example.com/x.png", "https://site.test/page.html"),
            "https://cdn.example.com/x.png"
        );
    }

    #[test]
    fn absolute_and_data_uris_pass_through() {
        assert_eq!(
            to_absolute_url("https://other.test/a.png", "http://h/page.html"),
            "https://other.test/a.png"
        );
        let data = "data:image/png;base64,iVBORw0KGgo=";
        assert!(to_absolute_url(data, "http://h/page.html").starts_with("data:image/png;base64,"));
    }
}

#[cfg(test)]
mod band_cursor_tests {
    use super::{BandCursor, FloatBand};

    fn bands() -> Vec<FloatBand> {
        vec![
            FloatBand {
                left_inset: 0.0,
                line_width: 400.0,
                height: Some(100.0),
            },
            FloatBand {
                left_inset: 0.0,
                line_width: 600.0,
                height: None,
            },
        ]
    }

    #[test]
    fn a_band_holds_as_many_whole_lines_as_it_has_room_for() {
        let cursor = BandCursor::new(&bands());
        assert_eq!(cursor.lines_left(20.0), 5);
        // 4.5 lines' worth of room holds four whole ones; the fifth would cross the float.
        assert_eq!(cursor.lines_left(22.0), 4);
    }

    #[test]
    fn the_last_band_is_open_ended() {
        let mut cursor = BandCursor::new(&bands());
        cursor.take_lines(5, 20.0);
        assert_eq!(cursor.current().line_width, 600.0);
        assert_eq!(cursor.lines_left(20.0), usize::MAX);
    }

    #[test]
    fn the_unused_tail_of_a_band_is_owed_to_the_next_line() {
        // 100px of band at 22px per line: four lines fit and 12px are left over. The next line
        // must start below the float, not in the sliver - CSS moves a line box that would
        // intersect a float down until it clears.
        let mut cursor = BandCursor::new(&bands());
        assert_eq!(cursor.placement().offset_top, 0.0);
        cursor.take_lines(4, 22.0);
        assert!(cursor.advance() || cursor.current().height.is_none());
        let placement = cursor.placement();
        assert_eq!(placement.band.line_width, 600.0);
        assert_eq!(placement.offset_top, 12.0);
        // Only owed once.
        assert_eq!(cursor.placement().offset_top, 0.0);
    }

    #[test]
    fn filling_a_band_exactly_advances_without_a_gap() {
        let mut cursor = BandCursor::new(&bands());
        cursor.take_lines(5, 20.0);
        assert_eq!(cursor.placement().offset_top, 0.0);
    }

    #[test]
    fn exhausting_jumps_to_the_open_ended_band() {
        let mut cursor = BandCursor::new(&bands());
        cursor.exhaust();
        assert_eq!(cursor.current().line_width, 600.0);
        assert!(cursor.current().height.is_none());
    }
}

#[cfg(test)]
mod whitespace_tests {
    use super::{is_collapsible_whitespace, split_collapsible_whitespace};

    const NBSP: &str = "\u{a0}";

    #[test]
    fn source_formatting_is_collapsible() {
        assert!(is_collapsible_whitespace(" "));
        assert!(is_collapsible_whitespace("\n    "));
        assert!(is_collapsible_whitespace("\t\r\n"));
        assert!(is_collapsible_whitespace(""));
    }

    #[test]
    fn a_non_breaking_space_is_content() {
        // The bug in one line: `str::trim` and `char::is_whitespace` use the Unicode set, which
        // counts U+00A0, so a text node holding only an `&nbsp;` was discarded as indentation.
        // Parsoid gives every entity its own element, so Wikipedia's
        // `Designed<span>&nbsp;</span>by` lost its space and read "Designedby".
        assert!(!is_collapsible_whitespace(NBSP));
        assert!(!is_collapsible_whitespace(&format!(" {NBSP} ")));
        // The other fixed-width spaces are content too.
        assert!(!is_collapsible_whitespace("\u{2009}"));
        assert!(!is_collapsible_whitespace("\u{2007}"));
    }

    #[test]
    fn words_split_on_collapsible_whitespace_only() {
        assert_eq!(
            split_collapsible_whitespace("a b\n c").collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        // An nbsp binds its neighbours into one word - that is what it is for.
        let joined = format!("May{NBSP}1,");
        assert_eq!(
            split_collapsible_whitespace(&joined).collect::<Vec<_>>(),
            [joined.as_str()]
        );
        // And on its own it is a word, not a separator that vanishes.
        assert_eq!(split_collapsible_whitespace(NBSP).collect::<Vec<_>>(), [NBSP]);
        assert!(split_collapsible_whitespace("   ").next().is_none());
    }
}

#[cfg(test)]
mod leading_whitespace_tests {
    use crate::common::document::style::Display as CssDisplay;

    /// The rule the two call sites share, stated on its own: only a box that starts a line may
    /// trim whitespace at its start.
    fn starts_a_line(display: CssDisplay) -> bool {
        !matches!(display, CssDisplay::Inline)
    }

    #[test]
    fn a_block_starts_a_line_and_may_trim() {
        assert!(starts_a_line(CssDisplay::Block));
        assert!(starts_a_line(CssDisplay::TableCell));
        // An inline-block establishes its own formatting context, so its leading whitespace does
        // go - unlike an inline box, which merely continues the line it is on.
        assert!(starts_a_line(CssDisplay::InlineBlock));
    }

    #[test]
    fn an_inline_box_continues_a_line_and_must_not_trim() {
        // `1964<span>;</span><span> </span>62` - Parsoid gives every entity its own element, so a
        // lone space routinely *is* the whole content of one. Trimming it per element rather than
        // per line box rendered that as "1964;62", and cost every space between the tokens of a
        // syntax-highlighted code block, where each token is its own span.
        assert!(!starts_a_line(CssDisplay::Inline));
    }
}
