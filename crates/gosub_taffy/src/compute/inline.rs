use taffy::{
    AvailableSpace, CollapsibleMarginSet, Layout, LayoutInput, LayoutOutput, LayoutPartialTree,
    NodeId, Point, ResolveOrZero, RunMode, Size,
};

use crate::{LayoutDocument, TaffyLayouter};
use gosub_render_backend::layout::{CssProperty, LayoutTree, Node};
use parley::layout::{Alignment, GlyphRun};
use parley::style::{FontSettings, FontStack, FontStyle, FontVariation, FontWeight, StyleProperty};
use parley::swash::scale::ScaleContext;
use parley::{FontContext, InlineBox, LayoutContext};

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

    let mut str_buf = String::new();
    let mut text_node_data = Vec::new();
    let mut inline_boxes = Vec::new();

    for child in &children {
        let node_id = NodeId::from((*child).into());

        let Some(node) = tree.0.get_node(*child) else {
            continue;
        };

        if let Some(text) = node.text_data() {
            if text.is_empty() {
                continue;
            }

            let only_whitespace = text.chars().all(|c| c.is_whitespace());
            if only_whitespace {
                continue;
            }

            str_buf.push(' ');
            str_buf.push_str(text);

            let font_family = node
                .get_property("font-family")
                .and_then(|s| s.as_string())
                .map(|s| s.to_string())
                .unwrap_or("sans-serif".to_string());

            let font_size = node
                .get_property("font-size")
                .map(|s| s.unit_to_px())
                .unwrap_or(12.0);

            let alignment = parse_alignment(node);

            let font_weight = parse_font_weight(node);

            let font_style = parse_font_style(node);

            let var_axes = parse_font_axes(node);

            let line_height = node
                .get_property("line-height")
                .map(|s| s.unit_to_px())
                .unwrap_or(font_size * 1.2);

            let word_spacing = node
                .get_property("word-spacing")
                .map(|s| s.unit_to_px())
                .unwrap_or(0.0);

            let letter_spacing = node
                .get_property("letter-spacing")
                .map(|s| s.unit_to_px())
                .unwrap_or(0.0);

            text_node_data.push(TextNodeData {
                font_family,
                font_size,
                line_height,
                word_spacing,
                letter_spacing,
                alignment,
                font_weight,
                font_style,
                var_axes,

                to: str_buf.len(),
            });
        } else {
            let out = tree.compute_child_layout(node_id, layout_input);
            inline_boxes.push(InlineBox {
                id: node_id.into(),
                index: str_buf.len(),
                height: out.size.height, //TODO: handle inline-block => add margin & padding
                width: out.size.width,
            });
        }
    }

    println!("str_buf: {:?}", str_buf);
    println!("inline boxes: {:?}", inline_boxes.len());

    if inline_boxes.is_empty() && str_buf.is_empty() {
        return LayoutOutput::HIDDEN;
    }

    if !inline_boxes.is_empty() && str_buf.is_empty() {
        str_buf.push(0 as char);
    }
    let mut font_cx = FontContext::default();
    let mut layout_cx: LayoutContext<()> = LayoutContext::new();
    // let mut scale_cx = ScaleContext::new();

    let mut builder = layout_cx.ranged_builder(&mut font_cx, &str_buf, 1.0);
    let mut align = Alignment::default();

    if let Some(default) = text_node_data.first() {
        builder.push_default(&StyleProperty::FontStack(FontStack::Source(
            &default.font_family,
        )));
        builder.push_default(&StyleProperty::FontSize(default.font_size));
        builder.push_default(&StyleProperty::LineHeight(default.line_height));
        builder.push_default(&StyleProperty::WordSpacing(default.word_spacing));
        builder.push_default(&StyleProperty::LetterSpacing(default.letter_spacing));
        builder.push_default(&StyleProperty::FontWeight(default.font_weight));
        builder.push_default(&StyleProperty::FontStyle(default.font_style));
        builder.push_default(&StyleProperty::FontVariations(FontSettings::List(
            &default.var_axes,
        )));

        align = default.alignment;

        let mut from = default.to;

        for text_node in text_node_data.get(1..).unwrap_or_default() {
            builder.push(
                &StyleProperty::FontStack(FontStack::Source(&text_node.font_family)),
                from..text_node.to,
            );
            builder.push(
                &StyleProperty::FontSize(text_node.font_size),
                from..text_node.to,
            );
            builder.push(
                &StyleProperty::LineHeight(text_node.line_height),
                from..text_node.to,
            );
            builder.push(
                &StyleProperty::WordSpacing(text_node.word_spacing),
                from..text_node.to,
            );
            builder.push(
                &StyleProperty::LetterSpacing(text_node.letter_spacing),
                from..text_node.to,
            );
            builder.push(
                &StyleProperty::FontWeight(text_node.font_weight),
                from..text_node.to,
            );
            builder.push(
                &StyleProperty::FontStyle(text_node.font_style),
                from..text_node.to,
            );
            builder.push(
                &StyleProperty::FontVariations(FontSettings::List(&text_node.var_axes)),
                from..text_node.to,
            );

            from = text_node.to;
        }
    }

    for inline_box in inline_boxes {
        builder.push_inline_box(inline_box);
    }

    let mut layout = builder.build();

    let max_width = match layout_input.available_space.width {
        AvailableSpace::Definite(width) => Some(width),
        AvailableSpace::MinContent => Some(0.0),
        AvailableSpace::MaxContent => None,
    };

    layout.break_all_lines(max_width);

    layout.align(max_width, align);

    let width = layout.width();
    let height = layout.height();

    //
    // for (child, out) in children.into_iter().zip(outputs.into_iter()) {

    //     let node_id = NodeId::from(child.into());
    //
    //     let style = tree.get_style(node_id);
    //
    //     let location = Point {
    //         x: width,
    //         y: height - out.size.height,
    //     };
    //
    //     let border = style.border.resolve_or_zero(layout_input.parent_size);
    //     let padding = style.padding.resolve_or_zero(layout_input.parent_size);
    //
    //     width += out.size.width + border.left + border.right + padding.left + padding.right;
    //
    //     tree.set_unrounded_layout(
    //         node_id,
    //         &Layout {
    //             size: out.size,
    //             content_size: out.content_size,
    //             order: 0,
    //             location,
    //             border,
    //             padding,
    //             scrollbar_size: Size::ZERO, //TODO
    //         },
    //     );
    // }

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

struct TextNodeData {
    font_family: String,
    font_size: f32,
    line_height: f32,
    word_spacing: f32,
    letter_spacing: f32,
    alignment: Alignment,
    font_weight: FontWeight, // Axis: WGHT
    font_style: FontStyle,   // Axis: ITAL
    var_axes: Vec<FontVariation>,

    to: usize,
}

fn parse_alignment(node: &mut impl Node) -> Alignment {
    let Some(prop) = node.get_property("text-align") else {
        return Alignment::Start;
    };

    let Some(s) = prop.as_string() else {
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

fn parse_font_weight(node: &mut impl Node) -> FontWeight {
    let Some(prop) = node.get_property("font-weight") else {
        return FontWeight::NORMAL;
    };

    let Some(s) = prop.as_string() else {
        if let Some(v) = prop.as_number() {
            return FontWeight::new(v);
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

fn parse_font_style(node: &mut impl Node) -> FontStyle {
    let Some(prop) = node.get_property("font-style") else {
        return FontStyle::Normal;
    };

    let Some(s) = prop.as_string() else {
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

fn parse_font_axes(p: &mut impl Node) -> Vec<FontVariation> {
    _ = p;

    //TODO

    Vec::new()
}
