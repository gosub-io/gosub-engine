use crate::common::document::node::{Node, NodeId as DomNodeId, NodeType};
use crate::common::document::style::{lookup, FontWeight, StyleProperty, TextAlign, Unit, Value};
use crate::common::font::{FontAlignment, FontInfo};
use crate::common::geo;
use crate::common::geo::Coordinate;
use crate::common::media::MediaStore;
use crate::common::media::{Media, MediaId, MediaType};
use crate::layouter::box_model::Edges;
use crate::layouter::css_taffy_converter::CssTaffyConverter;
use crate::layouter::table::post_process_tables;
use crate::layouter::text::get_text_layout;
use crate::layouter::{
    box_model, BackgroundMedia, CanLayout, ElementContext, ElementContextImage, ElementContextSvg, ElementContextText,
    LayoutElementId, LayoutElementNode, LayoutTree,
};
use crate::rendertree_builder::{RenderNodeId, RenderTree};
use gosub_fontmanager::ParleyFontSystem;
use parking_lot::{Mutex, RwLock};
use std::borrow::Borrow;
use std::collections::HashMap;
use std::sync::Arc;
use taffy::prelude::*;
use taffy::NodeId as TaffyNodeId;

const DEFAULT_FONT_SIZE: f64 = 16.0;
const DEFAULT_FONT_FAMILY: &str = "sans-serif";

// Cache key: (text, font_family, size_bits, line_height_bits, weight, max_width_bits).
// Floats are stored as their bit pattern so the tuple is Hash + Eq.
type MeasureKey = (String, String, u32, u32, i32, u32);

/// Layouter structure that uses taffy as layout engine
pub struct TaffyLayouter {
    /// Generated taffy tree
    tree: TaffyTree<TaffyContext>,
    /// Root id of the taffy tree
    root_id: TaffyNodeId,
    /// Mapping of layout element id to taffy node id
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
    /// Shared font system used for text measurement. Locking once per measurement
    /// call avoids keeping the guard alive across the full layout pass, which lets
    /// other threads (e.g. the rasterizer) access the font collection between calls.
    font_system: Arc<Mutex<ParleyFontSystem>>,
    /// Memoized text measurements. Taffy calls the measure function 2-4× per node
    /// (MinContent, MaxContent, actual width). Caching by (text, font, max_width)
    /// eliminates the redundant Parley shaping calls.
    measure_cache: HashMap<MeasureKey, Size<f32>>,
    /// Reverse index: DOM node ID → layout element ID.
    /// Built during generate_taffy_element and used by the table post-processing pass.
    dom_to_layout_mapping: HashMap<DomNodeId, LayoutElementId>,
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

    fn image(src: &str, media_id: MediaId, dimension: geo::Dimension, node_id: DomNodeId) -> TaffyContext {
        TaffyContext::Image(ElementContextImage {
            node_id,
            src: src.to_string(),
            media_id,
            dimension,
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
    pub fn with_font_system(font_system: Arc<Mutex<ParleyFontSystem>>) -> Self {
        Self {
            tree: TaffyTree::new(),
            root_id: TaffyNodeId::new(0),
            layout_taffy_mapping: HashMap::new(),
            anon_container_map: HashMap::new(),
            media_store: Arc::new(MediaStore::new()),
            font_system,
            measure_cache: HashMap::new(),
            dom_to_layout_mapping: HashMap::new(),
        }
    }

    /// Expose the font system so callers can share it with other components.
    pub fn font_system(&self) -> Arc<Mutex<ParleyFontSystem>> {
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
        // let root_id = RenderNodeId::new(2);
        let mut layout_tree = self.generate_tree(render_tree, root_id);

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

        // Compute the layout with a measure function
        if let Err(e) = self
            .tree
            .compute_layout_with_measure(self.root_id, size, |v_kd, v_as, _v_ni, v_nc, _v_s| {
                // If taffy already knows both dimensions, no measurement needed.
                if let (Some(w), Some(h)) = (v_kd.width, v_kd.height) {
                    return Size { width: w, height: h };
                }

                match v_nc {
                    // Calculate text node
                    Some(TaffyContext::Text(text_ctx)) => {
                        let max_width = if text_ctx.no_wrap {
                            // white-space: nowrap — measure at unlimited width so text never wraps
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
                    // Return image intrinsic dimensions; CSS width/height constraints applied by taffy
                    Some(TaffyContext::Image(image_ctx)) => Size {
                        width: image_ctx.dimension.width as f32,
                        height: image_ctx.dimension.height as f32,
                    },
                    // SVG-backed <img> elements carry their intrinsic size the same way.
                    // Without this arm they measured as 0×0 and collapsed (e.g. the HN logo).
                    Some(TaffyContext::Svg(svg_ctx)) => Size {
                        width: svg_ctx.dimension.width as f32,
                        height: svg_ctx.dimension.height as f32,
                    },
                    _ => Size::ZERO,
                }
            })
        {
            log::error!("Failed to compute taffy layout: {:?}", e);
            self.measure_cache = measure_cache;
            return layout_tree;
        }
        self.measure_cache = measure_cache;

        // Since we are not interested in taffy layout after this stage in the pipeline, we convert
        // the taffy layout to a box model layout tree. This makes the rest of the pipeline
        // layout-engine agnostic.
        let root_id = layout_tree.root_id;
        let root_width = layout_tree.root_dimension.width;
        self.populate_boxmodel(&mut layout_tree, root_id, Coordinate::ZERO, root_width);
        post_process_tables(&mut layout_tree, &self.dom_to_layout_mapping);

        // get dimension of the root node
        if let Some(root) = layout_tree.get_node_by_id(root_id) {
            let w = root.box_model.margin_box.width as f32;
            let h = root.box_model.margin_box.height as f32;
            layout_tree.root_dimension = geo::Dimension::new(w as f64, h as f64);
        }

        layout_tree
    }
}

impl TaffyLayouter {
    // Populate the layout tree with the box models that we now can generate
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

        let Some(el) = layout_tree.get_node_by_id_mut(layout_node_id) else {
            log::warn!("Layout node {:?} not found in arena", layout_node_id);
            return;
        };
        el.box_model = taffy_layout_to_boxmodel(&layout, offset);
        // For text nodes, available_width is the wrap limit passed to the renderer.
        // Use the parent element's content width (supplied by our caller), which is the
        // most accurate available constraint for text that lives directly in a block box.
        if let ElementContext::Text(ref mut text_ctx) = el.context {
            text_ctx.available_width = parent_content_width;
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

        // Absolute position of this node's content area — used as the base offset for direct children.
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

    /// Generate the layout tree from the render tree
    fn generate_tree(&mut self, render_tree: RenderTree, root_id: RenderNodeId) -> LayoutTree {
        self.measure_cache.clear();
        self.tree = TaffyTree::new();
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

    // Process inline elements by adding them to the taffy tree and element_node children vec. It will
    // automatically create an anonymous taffy block element to store multiple inline elements.
    fn process_inlines(
        &mut self,
        current_inline_group: &Vec<(LayoutElementId, TaffyNodeId)>,
        element_node: &mut LayoutElementNode,
        leaf_id: TaffyNodeId,
    ) {
        log::debug!("Processing inline elements: {:?}", current_inline_group.len());

        // No inline elements to process
        if current_inline_group.is_empty() {
            return;
        }

        // All inline elements (even a single one) are wrapped in an anonymous flex container.
        // This ensures the text measure function always receives AvailableSpace::Definite from
        // the flex algorithm, preventing single-child text nodes from getting MaxContent width
        // (which would make them lay out on one line and overflow their block parent).
        let Ok(taffy_container_id) = self.tree.new_leaf(Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_self: Some(AlignSelf::FlexStart),
            // FlexStart ensures multi-row intrinsic height = sum of all row heights.
            // Taffy's default (None = Stretch) fails to include wrapped rows in the
            // container's auto height, causing rows beyond the first to overflow.
            align_content: Some(AlignContent::FlexStart),
            gap: Size {
                width: LengthPercentage::length(0.0),
                height: LengthPercentage::length(0.0),
            },
            size: Size {
                width: Dimension::auto(),
                height: Dimension::auto(),
            },
            ..Default::default()
        }) else {
            return;
        };
        if let Err(e) = self.tree.add_child(leaf_id, taffy_container_id) {
            log::warn!("Failed to add anonymous container to taffy tree: {:?}", e);
        }

        // and add all the inline elements to the anonymous element
        for (inline_layout_element_id, inline_taffy_node_id) in current_inline_group {
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

    // Process node and turn it into a taffy node. It will recursively process any children and takes care to wrap any multiple inline elements
    // into an anonymous taffy block element. This way we can sort of emulate inline elements within taffy.
    fn generate_taffy_element(
        &mut self,
        layout_tree: &mut LayoutTree,
        render_node_id: RenderNodeId,
    ) -> Option<(LayoutElementId, TaffyNodeId)> {
        // Find render node and dom node from the layout tree
        let render_node = layout_tree.render_tree.get_node_by_id(render_node_id)?;
        let dom_node = layout_tree
            .render_tree
            .doc
            .get_node_by_id(DomNodeId::from(render_node.node_id))?;

        // Extract taffy data from the DOM node
        let Some((taffy_context, taffy_style)) = self.extract_taffy_data(layout_tree, &dom_node) else {
            // Could not extract taffy data from the DOM node
            return None;
        };

        // Flex and grid containers are formatting contexts where ALL children — inline or block —
        // are direct layout participants. Wrapping inline children in an anonymous flex container
        // would insert an extra level that breaks the parent's `gap`, `align-items`, etc.
        let parent_is_flex_or_grid = matches!(taffy_style.display, Display::Flex | Display::Grid);

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
            // Could not create a leaf node in the taffy tree
            return None;
        };

        let background_media = self.resolve_background_media(layout_tree, dom_node.node_id);

        let mut element_node = LayoutElementNode {
            id: layout_tree.next_node_id(),
            dom_node_id: dom_node.node_id,
            render_node_id,
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
        let render_node_children = render_node.children.clone();
        for child_id in render_node_children.iter() {
            let Some((child_layout_element_id, child_taffy_id)) = self.generate_taffy_element(layout_tree, *child_id)
            else {
                continue;
            };

            let Some(child_node) = layout_tree.render_tree.get_document_node_by_render_id(*child_id) else {
                continue;
            };

            // In a flex/grid parent every child is a direct layout participant — inline or block —
            // so skip the anonymous-container wrapping and add them straight to the parent.
            if parent_is_flex_or_grid {
                // Still discard pure-whitespace text nodes; they carry no visual content.
                if let NodeType::Text(text) = &child_node.node_type {
                    if text.trim().is_empty() {
                        // Drop leading whitespace (before any inline sibling). Keep inter-element
                        // whitespace — it collapses to a single space in extract_taffy_data and
                        // visually separates adjacent inline elements (e.g. between </span><span>).
                        if current_inline_group.is_empty() {
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
                let is_ws = if let NodeType::Text(text) = &child_node.node_type {
                    if text.trim().is_empty() {
                        // Drop leading whitespace (before any inline sibling). Keep inter-element
                        // whitespace — it collapses to a single space in extract_taffy_data and
                        // visually separates adjacent inline elements (e.g. between </span><span>).
                        if current_inline_group.is_empty() {
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
                current_inline_group.push((child_layout_element_id, child_taffy_id));
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
            self.process_inlines(&current_inline_group, &mut element_node, leaf_id);
            current_inline_group = Vec::new();
            trailing_ws_count = 0;

            if let Err(e) = self.tree.add_child(leaf_id, child_taffy_id) {
                log::warn!("Failed to add child to taffy tree: {:?}", e);
            }
            element_node.children.push(child_layout_element_id);
        }

        // Strip trailing whitespace and deal with any remaining inline elements
        current_inline_group.truncate(current_inline_group.len().saturating_sub(trailing_ws_count));
        self.process_inlines(&current_inline_group, &mut element_node, leaf_id);

        // The layout-tree is the structure handed to the rest of the pipeline; taffy stays
        // internal to this layouter so other layout engines can be swapped in.
        let layout_element_id = element_node.id;
        layout_tree.arena.insert(layout_element_id, element_node);

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
        let media_id = match self.media_store.load_media(&abs) {
            Ok(media_id) => media_id,
            Err(e) => {
                log::warn!("Could not load background-image '{}': {}", abs, e);
                return None;
            }
        };

        // Pick the renderer based on what was actually stored: an SVG (e.g. HN's
        // `triangle.svg` votearrow) must go through the SVG paint path, not a raster blit.
        match &*self.media_store.get(media_id, MediaType::Image) {
            Media::Svg(_) => Some(BackgroundMedia::Svg(media_id)),
            Media::Image(_) => Some(BackgroundMedia::Image(media_id)),
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

                // Images get a taffy context so their intrinsic size participates in layout.
                if data.tag_name.eq_ignore_ascii_case("img") {
                    let base_url = layout_tree.render_tree.doc.base_url();
                    let Some(src) = data.get_attribute("src") else {
                        log::warn!("img element missing src attribute");
                        return None;
                    };
                    let src = to_absolute_url(src, &base_url);

                    log::debug!("Loading (image) resource: {}", src);

                    let Ok(media_id) = self.media_store.load_media(src.as_str()) else {
                        // Could not load media
                        log::warn!("Could not load media from path: {}", src);
                        return None;
                    };

                    let media = self.media_store.get(media_id, MediaType::Image);
                    // When the media is a placeholder (load failed), use a small fixed
                    // size so the broken-image icon doesn't blow up the layout. The
                    // rasterizer scales the icon to whatever rect the element actually
                    // occupies, so display quality is unaffected.
                    let is_placeholder = self.media_store.is_placeholder(media_id);
                    taffy_context = match media.borrow() {
                        Media::Svg(media_svg) => {
                            // Use the SVG's intrinsic size so the element gets a non-zero box.
                            // A failed/placeholder load uses the same small fixed size as images.
                            let dimension = if is_placeholder {
                                geo::Dimension::new(32.0, 32.0)
                            } else {
                                let size = media_svg.svg.tree.size();
                                geo::Dimension::new(size.width() as f64, size.height() as f64)
                            };
                            Some(TaffyContext::svg(src.as_str(), media_id, dimension, dom_node.node_id))
                        }
                        Media::Image(media_image) => {
                            let dimension = if is_placeholder {
                                geo::Dimension::new(32.0, 32.0)
                            } else {
                                geo::Dimension::new(media_image.image.width() as f64, media_image.image.height() as f64)
                            };
                            Some(TaffyContext::image(src.as_str(), media_id, dimension, dom_node.node_id))
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

                // Default font
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

                let alignment = match doc.get_style(dom_node.node_id, &StyleProperty::TextAlign) {
                    Value::TextAlign(value) => match value {
                        TextAlign::Center => FontAlignment::Center,
                        TextAlign::End => FontAlignment::End,
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

                // Apply CSS white-space: normal — collapse newlines/runs of whitespace to a
                // single space and strip leading/trailing whitespace.  Raw HTML text nodes
                // contain the literal source indentation (e.g. "\n    Red box…\n  ") which
                // pango would render as a blank first line if left untouched.
                // Whitespace-only source nodes (e.g. "\n  " between </span><span>) collapse
                // to a single space so they produce an inter-element gap when kept.
                let is_whitespace_only = !text.is_empty() && text.chars().all(|c: char| c.is_ascii_whitespace());
                // Preserve one leading/trailing inter-element gap as NBSP (non-breaking) so
                // pango does not wrap at the boundary space, while still rendering a visible gap.
                let had_leading_space = text.starts_with(|c: char| c.is_ascii_whitespace());
                let had_trailing_space = text.ends_with(|c: char| c.is_ascii_whitespace());
                let mut text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !is_whitespace_only {
                    if had_leading_space && !text.is_empty() {
                        text.insert(0, '\u{00A0}');
                    }
                    if had_trailing_space && !text.is_empty() {
                        text.push('\u{00A0}');
                    }
                }
                if is_whitespace_only {
                    // Inter-element whitespace (e.g. between </span><span>). Collapse to a single
                    // NBSP so the text context is non-empty. We bypass parley measurement entirely
                    // by setting an explicit taffy width (~0.3em), because parley returns 0 for
                    // spaces when called with MinContent (max_advance=0), causing the flex item to
                    // collapse. flex_shrink=0 prevents the space from being squeezed away.
                    text = "\u{00A0}".to_string();
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

                let text_decoration = match doc.get_style(dom_node.node_id, &StyleProperty::TextDecorationLine) {
                    Value::Keyword(id) => lookup(id),
                    _ => String::new(),
                };

                let font_info = FontInfo {
                    family: font_family,
                    size: font_size,
                    weight: font_weight as i32,
                    width: 100, // 100%, normal
                    slant: if font_italic { 1 } else { 0 },
                    line_height,
                    alignment,
                    underline: text_decoration.contains("underline"),
                    line_through: text_decoration.contains("line-through"),
                };

                taffy_context = Some(TaffyContext::text(
                    text.as_str(),
                    font_info,
                    dom_node.node_id,
                    text_offset,
                    no_wrap,
                ));

                // Whitespace-only separator nodes must not grow — they should remain the
                // natural width of a single space character so they don't consume the flex row.
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
        // Base URL unusable (e.g. empty for an inline document) — fall back to the raw reference.
        Err(_) => uri.to_string(),
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
    use super::to_absolute_url;

    #[test]
    fn relative_ref_replaces_base_last_segment() {
        // A relative reference resolves against the document, *replacing* the page file —
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
