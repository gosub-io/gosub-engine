use crate::common::document::node::{Node, NodeId as DomNodeId, NodeType};
use crate::common::document::style::{FontWeight, StyleProperty, StyleValue, TextAlign, Unit};
use crate::common::geo::Coordinate;
use crate::common::media::{Media, MediaId, MediaType};
use crate::common::{geo, get_media_store};
use crate::layouter::css_taffy_converter::CssTaffyConverter;
use crate::layouter::text::get_text_layout;
use crate::layouter::{
    box_model, CanLayout, ElementContext, ElementContextImage, ElementContextSvg,
    ElementContextText, LayoutElementId, LayoutElementNode, LayoutTree,
};
use crate::rendertree_builder::{RenderNodeId, RenderTree};
use std::borrow::Borrow;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use taffy::prelude::*;
use taffy::NodeId as TaffyNodeId;
use crate::common::font::{FontAlignment, FontInfo};
use crate::layouter::box_model::Edges;

const DEFAULT_FONT_SIZE: f64 = 16.0;
const DEFAULT_FONT_FAMILY: &str = "Sans";

/// Layouter structure that uses taffy as layout engine
pub struct TaffyLayouter {
    /// Generated taffy tree
    tree: TaffyTree<TaffyContext>,
    /// Root id of the taffy tree
    root_id: TaffyNodeId,
    /// Mapping of layout element id to taffy node id
    layout_taffy_mapping: HashMap<LayoutElementId, TaffyNodeId>,
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
    ) -> TaffyContext {
        TaffyContext::Text(ElementContextText {
            node_id,
            font_info,
            text: text.to_string(),
            text_offset,
        })
    }

    fn image(
        src: &str,
        media_id: MediaId,
        dimension: geo::Dimension,
        node_id: DomNodeId,
    ) -> TaffyContext {
        TaffyContext::Image(ElementContextImage {
            node_id,
            src: src.to_string(),
            media_id,
            dimension,
        })
    }

    fn svg(src: &str, media_id: MediaId, node_id: DomNodeId) -> TaffyContext {
        TaffyContext::Svg(ElementContextSvg {
            node_id,
            src: src.to_string(),
            media_id,
            dimension: geo::Dimension::ZERO,
        })
    }
}

impl TaffyLayouter {
    pub fn new() -> Self {
        Self {
            tree: TaffyTree::new(),
            root_id: TaffyNodeId::new(0),
            layout_taffy_mapping: HashMap::new(),
        }
    }

    pub fn print_tree(&mut self) {
        self.tree.print_tree(self.root_id);
    }
}

impl CanLayout for TaffyLayouter {
    fn layout(&mut self, render_tree: RenderTree, viewport: Option<geo::Dimension>, dpi_scale_factor: f32) -> LayoutTree {
        let root_id = render_tree.root_id.unwrap();
        // let root_id = RenderNodeId::new(2);
        let Some(mut layout_tree) = self.generate_tree(render_tree, root_id) else {
            panic!("Failed to generate root node render tree");
        };

        // // Compute the layout based on the viewport
        let size = match viewport {
            Some(viewport) => Size {
                width: AvailableSpace::Definite(viewport.width as f32),
                height: AvailableSpace::Definite(viewport.height as f32),
            },
            None => Size::MAX_CONTENT,
        };

        /// Compute the layout with a measure function
        self.tree
            .compute_layout_with_measure(self.root_id, size, |v_kd, v_as, v_ni, v_nc, v_s| {
                match v_nc {
                    // Calculate text node
                    Some(TaffyContext::Text(text_ctx)) => {
                        let max_width = match v_as.width {
                            AvailableSpace::Definite(width) => width as f64,
                            AvailableSpace::MaxContent => 1_000_000_000.0,      // f64::MAX doesn't work. Seems some kind of overflow. Same goes for f32::MAX
                            AvailableSpace::MinContent => 0.0,
                        };

                        // Calculate the text layout dimensions and return it to taffy
                        let text_layout = get_text_layout(text_ctx.text.as_str(), &text_ctx.font_info, max_width, dpi_scale_factor);
                        match text_layout {
                            Ok(text_layout) => Size {
                                width: text_layout.width as f32,        // Note that we cast width down to 32bits.. we might need to take care of overflows
                                height: text_layout.height as f32,
                            },
                            Err(_) => Size::ZERO,
                        }
                    }
                    _ => Size::ZERO,
                }
            })
            .unwrap();

        // Since we are not interested in taffy layout after this stage in the pipeline, we convert
        // the taffy layout to a box model layout tree. This makes the rest of the pipeline
        // layout-engine agnostic.
        let root_id = layout_tree.root_id;
        self.populate_boxmodel(&mut layout_tree, root_id, Coordinate::ZERO);

        // get dimension of the root node
        let root = layout_tree.get_node_by_id(root_id).unwrap();
        let w = root.box_model.margin_box.width as f32; // + 100.0;
        let h = root.box_model.margin_box.height as f32; // + 100.0;
        layout_tree.root_dimension = geo::Dimension::new(w as f64, h as f64);

        self.tree.print_tree(self.root_id);
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
    ) {
        let taffy_node_id = self.layout_taffy_mapping.get(&layout_node_id).unwrap();
        let layout = self.tree.layout(*taffy_node_id).unwrap().clone();

        let el = layout_tree.get_node_by_id_mut(layout_node_id).unwrap();
        el.box_model = taffy_layout_to_boxmodel(&layout, offset);
        let child_ids = el.children.clone();

        for child_id in child_ids {
            self.populate_boxmodel(
                layout_tree,
                child_id,
                Coordinate::new(
                    offset.x + layout.location.x as f64,
                    offset.y + layout.location.y as f64,
                ),
            );
        }
    }

    /// Generate the layout tree from the render tree
    fn generate_tree(
        &mut self,
        render_tree: RenderTree,
        root_id: RenderNodeId,
    ) -> Option<LayoutTree> {
        self.tree = TaffyTree::new();
        self.root_id = TaffyNodeId::new(0); // Will be filled in later

        let mut layout_tree = LayoutTree {
            render_tree,
            arena: HashMap::new(),
            root_id: LayoutElementId::new(0), // Will be filled in later
            next_node_id: Arc::new(RwLock::new(LayoutElementId::new(0))),
            root_dimension: geo::Dimension::ZERO,
            rstar_tree: rstar::RTree::new(),
        };

        let Some((layout_element_root_id, taffy_root_id)) =
            self.generate_taffy_element(&mut layout_tree, root_id)
        else {
            return None;
        };

        layout_tree.root_id = layout_element_root_id;
        self.root_id = taffy_root_id;

        Some(layout_tree)
    }


    // Process inline elements by adding them to the taffy tree and element_node children vec. It will
    // automatically create an anonymous taffy block element to store multiple inline elements.
    fn process_inlines(
        &mut self,
        current_inline_group: &Vec<(LayoutElementId, TaffyNodeId)>,
        element_node: &mut LayoutElementNode,
        leaf_id: TaffyNodeId,
    ) {
        println!("Processing inline elements: {:?}", current_inline_group.len());

        // No inline elements to process
        if current_inline_group.is_empty() {
            return;
        }

        // We only have one inline element in the queue so we can add it directly to the taffy node
        if current_inline_group.len() == 1 {
            if let Some((inline_layout_element_id, inline_taffy_node_id)) = current_inline_group.first() {
                self.tree.add_child(leaf_id, *inline_taffy_node_id).unwrap();
                element_node.children.push(*inline_layout_element_id);
            }
            return;
        }

        // Multiple inline elements are found. We need to wrap them in an anonymous taffy block element
        let Ok(taffy_container_id) = self.tree.new_leaf(Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_self: Some(AlignSelf::FlexStart),
            gap: Size {
                width: LengthPercentage::Length(0.0),
                height: LengthPercentage::Length(0.0),
            },
            size: Size {
                width: Dimension::Auto,
                height: Dimension::Auto,
            },
            .. Default::default()
        }) else {
            return;
        };
        self.tree.add_child(leaf_id, taffy_container_id).unwrap();

        // and add all the inline elements to the anonymous element
        for (inline_layout_element_id, inline_taffy_node_id) in current_inline_group {
            self.tree.add_child(taffy_container_id, *inline_taffy_node_id);
            element_node.children.push(*inline_layout_element_id);
        }
    }


    // Process node and turn it into a taffy node. It will recursively process any children and takes care to wrap any multiple inline elements
    // into an anonymous taffy block element. This way we can sort of emulate inline elements within taffy.
    fn generate_taffy_element(&mut self, layout_tree: &mut LayoutTree, render_node_id: RenderNodeId) -> Option<(LayoutElementId, TaffyNodeId)> {
        // Find render node and dom node from the layout tree
        let Some(render_node) = layout_tree.render_tree.get_node_by_id(render_node_id) else {
            return None;
        };
        let Some(dom_node) = layout_tree
            .render_tree
            .doc
            .get_node_by_id(DomNodeId::from(render_node.node_id))
        else {
            return None;
        };

        // Extract taffy data from the DOM node
        let Some((taffy_context, taffy_style)) = self.extract_taffy_data(
            &layout_tree,
            &dom_node,
        ) else {
            // Could not extract taffy data from the DOM node
            return None;
        };

        // The context will be moved to the taffy tree, so we need to convert it before that happens.
        let element_context = match taffy_context {
            Some(ref ctx) => to_element_context(Some(&ctx)),
            None => to_element_context(None),
        };

        // We add the element to the taffy tree. If we have a context, make sure we add it with the context as well.
        let result = match taffy_context {
            Some(ctx) => self
                .tree
                .new_leaf_with_context(taffy_style.to_owned(), ctx),
            None => self.tree.new_leaf(taffy_style.to_owned()),
        };

        let Ok(leaf_id) = result else {
            // Could not create a leaf node in the taffy tree
            return None;
        };

        // Create the element node in our layout tree
        let mut element_node = LayoutElementNode {
            id: layout_tree.next_node_id(),
            dom_node_id: dom_node.node_id,
            render_node_id,
            box_model: box_model::BoxModel::ZERO,
            children: vec![],
            context: element_context,
        };

        // Now we iterate the children, and add them to both taffy and the element_node's children vec.

        let mut current_inline_group = Vec::new();
        let render_node_children = render_node.children.clone();
        for child_id in render_node_children.iter() {
            let Some((child_layout_element_id, child_taffy_id)) =
                self.generate_taffy_element(layout_tree, *child_id)
            else {
                continue;
            };

            let Some(child_node) = layout_tree.render_tree.get_document_node_by_render_id(*child_id) else {
                continue;
            };

            // Don't add inline elements to the taffy tree yet. We need to group them first and possibly wrap inside a block
            if child_node.is_inline_element() || child_node.is_text() {
                println!("Pushing element as inline: {:?}", child_node.node_id);
                current_inline_group.push((child_layout_element_id, child_taffy_id));
                continue;
            }

            println!("Element {:?} is not an inline", child_node.node_id);

            self.process_inlines(
                &mut current_inline_group,
                &mut element_node,
                leaf_id,
            );
            current_inline_group = Vec::new();

            // Add this child
            self.tree.add_child(leaf_id, child_taffy_id).unwrap();
            element_node.children.push(child_layout_element_id);
        }

        // Deal with any remaining inline elements in the current inline group
        self.process_inlines(
            &mut current_inline_group,
            &mut element_node,
            leaf_id,
        );

        // Finally, we can insert the generated element also in the layout-tree. This is the ultimate
        // structure we must return to the rest of the pipeline. Taffy itself will stay internal to this
        // layouter, as other layout engines can use different systems (or we can write our own as well)

        // Insert element node into our arena
        let layout_element_id = element_node.id;
        layout_tree.arena.insert(layout_element_id, element_node);

        /// Create a mapping between the layout element id and the taffy node id. We need this to generate
        /// the boxmodel at a later time in this pipeline stage.
        self.layout_taffy_mapping.insert(layout_element_id, leaf_id);

        Some((layout_element_id, leaf_id))
    }

    /// Extracts taffy variables based the DOM node. It will generate the taffy style based on the node CSS properties,
    /// any context that might be needed (images, svg, text).
    fn extract_taffy_data(&self, layout_tree: &LayoutTree, dom_node: &&Node) -> Option<(Option<TaffyContext>, Style)> {
        let mut taffy_context = None;
        let mut taffy_style = Style::default();

        match &dom_node.node_type {
            // Node is an element node (like a div, span, etc.)
            NodeType::Element(data) => {
                // Create the taffy style from our CSS and push it into the stack
                let conv = CssTaffyConverter::new(&data.styles);
                taffy_style = conv.convert(dom_node.node_id, false);

                // Check if element type is an image, if so, set the taffy context
                if data.tag_name.eq_ignore_ascii_case("img") {
                    let base_url = layout_tree.render_tree.doc.base_url();
                    let src = data.get_attribute("src").unwrap();
                    let src = to_absolute_url(src, base_url);

                    println!("Loading (image) resource: {}", src);

                    let media_store = get_media_store();
                    let Ok(media_id) = media_store.read().unwrap().load_media(src.as_str()) else {
                        // Could not load media
                        log::info!("Could not load media from path: {}", src);
                        return None;
                    };

                    let media_store = get_media_store();
                    let binding = media_store.read().unwrap();
                    let media = binding.get(media_id, MediaType::Image);
                    taffy_context = match media.borrow() {
                        Media::Svg(_) => {
                            Some(TaffyContext::svg(src.as_str(), media_id, dom_node.node_id))
                        }
                        Media::Image(media_image) => {
                            let dimension = geo::Dimension::new(
                                media_image.image.width() as f64,
                                media_image.image.height() as f64,
                            );
                            Some(TaffyContext::image(
                                src.as_str(),
                                media_id,
                                dimension,
                                dom_node.node_id,
                            ))
                        }
                    }
                }

                if data.tag_name.eq_ignore_ascii_case("svg") {
                    let inner_html = layout_tree.render_tree.doc.inner_html(dom_node.node_id);

                    let store = get_media_store();
                    match store
                        .read()
                        .unwrap()
                        .load_media_from_data(MediaType::Svg, inner_html.into_bytes().as_slice())
                    {
                        Ok(media_id) => {
                            taffy_context = Some(TaffyContext::svg(
                                "gosub://internal",
                                media_id,
                                dom_node.node_id,
                            ));
                        }
                        Err(e) => {
                            log::info!("Could not load SVG media: {:?}", e);
                        }
                    }
                }
            }
            NodeType::Text(text, node_style) => {
                let parent_node = match dom_node.parent_id {
                    Some(parent_id) => layout_tree.render_tree.doc.get_node_by_id(parent_id),
                    None => None,
                };
                if parent_node.is_none() {
                    return None;
                }

                // Default font
                let mut font_size = DEFAULT_FONT_SIZE;
                let mut font_family = DEFAULT_FONT_FAMILY.to_string();

                match node_style.get_property(StyleProperty::FontSize) {
                    Some(StyleValue::Unit(value, unit)) => match unit {
                        Unit::Px => font_size = *value as f64,
                        Unit::Em => panic!("Don't know how to deal with em units for fonts"),
                        Unit::Rem => panic!("Don't know how to deal with rem units for fonts"),
                        _ => panic!("Incorrect font-size property unit"),
                    },
                    _ => {}
                }

                match node_style.get_property(StyleProperty::FontFamily) {
                    Some(StyleValue::Keyword(value)) => font_family = value.clone(),
                    _ => {}
                }

                let font_weight = match node_style.get_property(StyleProperty::FontWeight) {
                    Some(StyleValue::FontWeight(weight)) => match weight {
                        FontWeight::Normal => 400.0,
                        FontWeight::Bold => 700.0,
                        FontWeight::Number(value) => *value as f64,
                        FontWeight::Bolder => {
                            unimplemented!("FontWeight::Bolder is not implemented yet")
                        }
                        FontWeight::Lighter => {
                            unimplemented!("FontWeight::Lighter is not implemented yet")
                        }
                    },
                    _ => 400.0,
                };

                let alignment = match node_style.get_property(StyleProperty::TextAlign) {
                    Some(StyleValue::TextAlign(value)) => match value {
                        TextAlign::Center => FontAlignment::Center,
                        TextAlign::Right => FontAlignment::Start,
                        TextAlign::Left => FontAlignment::End,
                        TextAlign::Justify => FontAlignment::Justify,
                        TextAlign::Start => FontAlignment::Start,
                        TextAlign::End => FontAlignment::End,
                        TextAlign::MatchParent => {
                            unimplemented!("TextAlign::MatchParent is not implemented yet")
                        }
                        TextAlign::Initial => {
                            unimplemented!("TextAlign::Initial is not implemented yet")
                        }
                        TextAlign::Inherit => {
                            unimplemented!("TextAlign::Inherit is not implemented yet")
                        }
                        TextAlign::Revert => {
                            unimplemented!("TextAlign::Revert is not implemented yet")
                        }
                        TextAlign::Unset => {
                            unimplemented!("TextAlign::Unset is not implemented yet")
                        }
                    },
                    _ => FontAlignment::Start,
                };

                let line_height = match node_style.get_property(StyleProperty::LineHeight) {
                    Some(StyleValue::Unit(value, unit)) => match unit {
                        Unit::Px => *value as f64,
                        Unit::Em => panic!("Don't know how to deal with em units for line-height"),
                        Unit::Rem => {
                            panic!("Don't know how to deal with rem units for line-height")
                        }
                        _ => panic!("Incorrect line-height property unit"),
                    },
                    _ => font_size,
                };

                // Calculate vertical offset for centering based on the line height.
                let text_offset = Coordinate::new(0.0, (line_height - font_size) / 2.0);

                let mut text = text.clone();
                // if inline_element_counter > 0 {
                //     // If we are in an inline container, we need to add a space between the text nodes
                //     text = format!(" {}", text).clone()
                // }

                let font_info = FontInfo {
                    family: font_family,
                    size: font_size,
                    weight: font_weight as i32,
                    width: 100, // 100%, normal
                    slant: 0,
                    line_height,
                    alignment,
                };

                taffy_context = Some(TaffyContext::text(
                    text.as_str(),
                    font_info,
                    dom_node.node_id,
                    text_offset,
                ));
            }
            NodeType::Comment(_) => {
                // No need to layout for comment nodes. In fact, they should have been removed already
                // by the render-tree building stage.
                return None;
            }
        }

        Some((
            taffy_context,
            taffy_style,
        ))
    }
}

// Convert a URI to an absolute URL based on the base URL if this is needed
fn to_absolute_url(uri: &str, base_uri: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }

    // We have a relative path, so we need to prepend the base URL
    // Make sure we don't have double slashes
    if base_uri.ends_with("/") && uri.starts_with("/") {
        return format!("{}{}", base_uri, &uri[1..]).to_string();
    }

    // Neither has a /
    if !base_uri.ends_with("/") && !uri.starts_with("/") {
        return format!("{}/{}", base_uri, uri).to_string();
    }

    format!("{}{}", base_uri, uri).to_string()
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
        ),
        Some(TaffyContext::Image(image_ctx)) => ElementContext::image(
            image_ctx.src.as_str(),
            image_ctx.media_id,
            image_ctx.dimension.clone(),
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

#[inline(always)]
fn dpi(value: f64) -> f64 {
    let dpi_scale_factor = 3.0;
    value * dpi_scale_factor
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
