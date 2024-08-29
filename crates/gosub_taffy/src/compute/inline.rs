use taffy::{
    AvailableSpace, CollapsibleMarginSet, Layout, LayoutInput, LayoutOutput, LayoutPartialTree,
    NodeId, Point, ResolveOrZero, RunMode, Size,
};

use gosub_render_backend::layout::{CssProperty, LayoutTree, Node};
use parley::layout::Alignment;
use parley::style::{FontStyle, FontVariation, FontWeight};
use parley::InlineBox;

use crate::{LayoutDocument, TaffyLayouter};

pub fn compute_inline_layout<LT: LayoutTree<TaffyLayouter>>(
    tree: &mut LayoutDocument<LT>,
    nod_id: LT::NodeId,
    mut layout_input: LayoutInput,
) -> LayoutOutput {
    layout_input.known_dimensions = Size::NONE;
    layout_input.run_mode = RunMode::PerformLayout; //TODO: We should respect the run mode

    let Some(children) = tree.0.children(nod_id) else {
        return LayoutOutput::HIDDEN;
    };

    let mut outputs = Vec::with_capacity(children.len());

    let mut str_buf = String::new();
    let mut text_node_data = Vec::new();
    let mut inline_boxes = Vec::new();

    let mut height = 0.0f32;
    for child in &children {
        let node_id = NodeId::from((*child).into());

        let Some(node) = tree.0.get_node(node_id) else {
            continue;
        };

        if let Some(text) = node.text_data() {
            str_buf.push(' ');
            str_buf.push_str(text.value());

            let font_stack = FontStack::parse(node.get_property("font-family"));
            let font_size = node
                .get_property("font-size")
                .map(|s| s.unit_to_px())
                .unwrap_or(12.0);

            let alignment = parse_alignment(node.get_property("text-align"));

            let color = node.get_property("color"); //TODO: how can we represent an brush here?

            let font_weight = parse_font_weight(node.get_property("font-weight"));

            let font_style = parse_font_style(node.get_property("font-style"));

            let var_axes = parse_font_axes(node.get_property("font-variation-settings"));

            text_node_data.push(TextNodeData {
                font_stack,
                font_size,
                alignment,
                color,
                font_weight,
                font_style,
                var_axes,
            });
        } else {
            let out = tree.compute_child_layout(node_id, layout_input);
            inline_boxes.push(InlineBox {
                id: node_id,
                index: str_buf.len(),
                height: out.size.height, //TODO: handle inline-block => add margin & padding
                width: out.size.width,
            });
        }
    }

    let mut width = 0.0f32;
    for (child, out) in children.into_iter().zip(outputs.into_iter()) {
        let node_id = NodeId::from(child.into());

        let style = tree.get_style(node_id);

        let location = Point {
            x: width,
            y: height - out.size.height,
        };

        let border = style.border.resolve_or_zero(layout_input.parent_size);
        let padding = style.padding.resolve_or_zero(layout_input.parent_size);

        width += out.size.width + border.left + border.right + padding.left + padding.right;

        tree.set_unrounded_layout(
            node_id,
            &Layout {
                size: out.size,
                content_size: out.content_size,
                order: 0,
                location,
                border,
                padding,
                scrollbar_size: Size::ZERO, //TODO
            },
        );
    }

    let content_size = Size { width, height };

    let mut size = content_size;

    if let AvailableSpace::Definite(width) = layout_input.available_space.width {
        size.width = size.width.min(width);
    }

    if let AvailableSpace::Definite(height) = layout_input.available_space.height {
        size.height = size.height.min(height);
    }

    LayoutOutput {
        size,
        content_size,
        first_baselines: Point::NONE,
        top_margin: CollapsibleMarginSet::ZERO,
        bottom_margin: CollapsibleMarginSet::ZERO,
        margins_can_collapse_through: false,
    }
}

struct TextNodeData<B> {
    font_stack: FontStack,
    font_size: f32,
    alignment: Alignment,
    color: B,
    font_weight: FontWeight, // Axis: WGHT
    font_style: FontStyle,   // Axis: ITAL
    var_axes: FontVariation,
}

struct FontStack {
    stack: Vec<String>,
}

impl FontStack {
    fn parse(s: &impl CssProperty) -> Self {
        let Some(s) = s.as_string() else {
            return Self {
                stack: vec!["sans-serif".to_string()],
            };
        };

        let mut stack = Vec::new();
        for family in s.split(',') {
            stack.push(family.trim().to_string());
        }
        Self { stack }
    }
}
fn parse_alignment(s: &impl CssProperty) -> Alignment {
    let Some(s) = s.as_string() else {
        return Alignment::Start;
    };

    match s {
        "left" => Alignment::Start,
        "center" => Alignment::Middle,
        "right" => Alignment::End,
        "justify" => Alignment::Justified,
        _ => Alignment::Start,
    }
}

fn parse_font_weight(s: &impl CssProperty) -> FontWeight {
    let Some(s) = s.as_string() else {
        if let Some(v) = s.as_number() {
            return v.into();
        };

        return FontWeight::NORMAL;
    };

    match s {
        "thin" => FontWeight::THIN,
        "extra-light" => FontWeight::EXTRA_LIGHT,
        "light" => FontWeight::LIGHT,
        "semi-light" => FontWeight::SEMI_LIGHT,
        "normal" => FontWeight::NORMAL,
        "medium" => FontWeight::MEDIUM,
        "semi-bold" => FontWeight::SEMI_BOLD,
        "bold" => FontWeight::BOLD,
        "extra-bold" => FontWeight::EXTRA_BOLD,
        "black" => FontWeight::BLACK,
        "extra-black" => FontWeight::EXTRA_BLACK,
        _ => FontWeight::NORMAL,
    }
}

fn parse_font_style(s: &impl CssProperty) -> FontStyle {
    let Some(s) = s.as_string() else {
        //TODO handle font-style: oblique

        return FontStyle::Normal;
    };

    match s {
        "normal" => FontStyle::Normal,
        "italic" => FontStyle::Italic,
        "oblique" => FontStyle::Italic,
        _ => FontStyle::Normal,
    }
}

fn parse_font_axes(p: &impl CssProperty) -> FontVariation {
    _ = p;

    //TODO

    FontVariation::default()
}
