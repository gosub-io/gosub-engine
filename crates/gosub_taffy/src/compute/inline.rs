use std::sync::{LazyLock, Mutex};

use log::warn;
use parley::layout::{Alignment, PositionedLayoutItem};
use parley::style::{FontSettings, FontStack, FontStyle, FontVariation, FontWeight, StyleProperty};
use parley::{FontContext, InlineBox, LayoutContext};
use taffy::{
    AvailableSpace, CollapsibleMarginSet, Layout, LayoutInput, LayoutOutput, LayoutPartialTree,
    NodeId, Point, Rect, RunMode, Size,
};

use gosub_render_backend::geo;
use gosub_render_backend::layout::{CssProperty, HasTextLayout, LayoutTree, Node};
use gosub_typeface::font::Glyph;

use crate::text::{Font, TextLayout};
use crate::{Display, LayoutDocument, TaffyLayouter};

static FONT_CX: LazyLock<Mutex<FontContext>> = LazyLock::new(|| Mutex::new(FontContext::default()));

pub fn compute_inline_layout<LT: LayoutTree<TaffyLayouter>>(
    tree: &mut LayoutDocument<LT>,
    nod_id: LT::NodeId,
    mut layout_input: LayoutInput,
) -> LayoutOutput {
    layout_input.known_dimensions = Size::NONE;
    layout_input.run_mode = RunMode::PerformLayout; //TODO: We should respect the run mode
                                                    // layout_input.sizing_mode = SizingMode::ContentSize;

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

            let line_height = node.get_property("line-height").and_then(|s| s.as_number());

            let word_spacing = node.get_property("word-spacing").map(|s| s.unit_to_px());

            let letter_spacing = node.get_property("letter-spacing").map(|s| s.unit_to_px());

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
                id: node_id,
            });
        } else {
            let out = tree.compute_child_layout(node_id, layout_input);

            tree.update_style(*child);

            let size = if let Some(cache) = tree.0.get_cache(*child) {
                if cache.display == Display::Inline {
                    //TODO: handle margins here

                    out.content_size
                } else {
                    out.size
                }
            } else {
                out.content_size
            };

            inline_boxes.push(InlineBox {
                id: node_id.into(),
                index: str_buf.len(),
                height: size.height,
                width: size.width,
            });
        }
    }

    if inline_boxes.is_empty() && str_buf.is_empty() {
        return LayoutOutput::HIDDEN;
    }

    if str_buf.is_empty() {
        str_buf.push(0 as char);
    }

    let mut layout_cx: LayoutContext<()> = LayoutContext::new();
    // let mut scale_cx = ScaleContext::new();

    let Ok(mut lock) = FONT_CX.lock() else {
        warn!("Failed to get font context");
        return LayoutOutput::HIDDEN;
    };
    let mut builder = layout_cx.ranged_builder(&mut lock, &str_buf, 1.0);
    let mut align = Alignment::default();

    if let Some(default) = text_node_data.first() {
        builder.push_default(&StyleProperty::FontStack(FontStack::Source(
            &default.font_family,
        )));
        builder.push_default(&StyleProperty::FontSize(default.font_size));
        if let Some(line_height) = default.line_height {
            builder.push_default(&StyleProperty::LineHeight(line_height));
        }
        if let Some(word_spacing) = default.word_spacing {
            builder.push_default(&StyleProperty::WordSpacing(word_spacing));
        }
        if let Some(letter_spacing) = default.letter_spacing {
            builder.push_default(&StyleProperty::LetterSpacing(letter_spacing));
        }
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
            if let Some(line_height) = text_node.line_height {
                builder.push(&StyleProperty::LineHeight(line_height), from..text_node.to);
            }
            if let Some(word_spacing) = text_node.word_spacing {
                builder.push(
                    &StyleProperty::WordSpacing(word_spacing),
                    from..text_node.to,
                );
            }
            if let Some(letter_spacing) = text_node.letter_spacing {
                builder.push(
                    &StyleProperty::LetterSpacing(letter_spacing),
                    from..text_node.to,
                );
            }
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

    let content_size = Size {
        width: layout.width().ceil(),
        height: layout.height().ceil(),
    };

    let mut current_node_idx = 0;
    let mut current_node_id = LT::NodeId::from(0);
    let mut current_to = 0;

    if let Some(first) = text_node_data.first() {
        current_node_id = LT::NodeId::from(first.id.into());
        current_to = first.to;
    }

    let mut current_glyph_idx = 0;

    'lines: for line in layout.lines() {
        let metrics = line.metrics();

        let height = metrics.line_height;

        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(run) => {
                    let mut offset = 0.0;

                    let grun = run.run();
                    let fs = grun.font_size();

                    let glyphs = run
                        .glyphs()
                        .map(|g| {
                            let gl = Glyph {
                                id: g.id,
                                x: g.x + offset,
                                y: g.y,
                            };

                            offset += g.advance;

                            gl
                        })
                        .collect::<Vec<_>>();

                    let run_y = run.baseline();

                    current_glyph_idx += glyphs.len();

                    if current_glyph_idx > current_to {
                        current_node_idx += 1;

                        if let Some(next) = text_node_data.get(current_node_idx) {
                            current_to = next.to;
                            current_node_id = LT::NodeId::from(next.id.into());
                        } else {
                            break 'lines;
                        }
                    }

                    let size = geo::Size {
                        width: run.advance(),
                        height,
                    };

                    let coords = grun.normalized_coords().to_owned();

                    let text_layout = TextLayout {
                        size,
                        font_size: fs,
                        font: Font(grun.font().clone()),
                        glyphs,
                        coords,
                    };

                    let Some(node) = tree.0.get_node(current_node_id) else {
                        continue;
                    };

                    node.set_text_layout(text_layout);

                    let size = Size {
                        width: size.width,
                        height: size.height,
                    };
                    tree.set_unrounded_layout(
                        NodeId::new(current_node_id.into()),
                        &Layout {
                            size,
                            content_size: size,
                            scrollbar_size: Size::ZERO,
                            border: Rect::ZERO,
                            location: Point {
                                x: run.offset(),
                                y: run_y,
                            },
                            order: 0,
                            padding: Rect::ZERO,
                        },
                    );
                }
                PositionedLayoutItem::InlineBox(inline_box) => {
                    let id = NodeId::from(inline_box.id);

                    let size = Size {
                        width: inline_box.width,
                        height: inline_box.height,
                    };

                    tree.set_unrounded_layout(
                        id,
                        &Layout {
                            size,
                            content_size: size,
                            scrollbar_size: Size::ZERO,
                            border: Rect::ZERO,
                            location: Point {
                                x: inline_box.x,
                                y: inline_box.y,
                            },
                            order: 0,
                            padding: Rect::ZERO,
                        },
                    );
                }
            }
        }
    }

    let mut size = content_size;

    if let AvailableSpace::Definite(width) = layout_input.available_space.width {
        size.width = content_size.width.min(width);
    }

    if let AvailableSpace::Definite(height) = layout_input.available_space.height {
        size.height = content_size.height.min(height);
    }

    LayoutOutput {
        size: content_size,
        content_size,
        first_baselines: Point::NONE,
        top_margin: CollapsibleMarginSet::ZERO,
        bottom_margin: CollapsibleMarginSet::ZERO,
        margins_can_collapse_through: false,
    }
}

#[derive(Debug)]
struct TextNodeData {
    font_family: String,
    font_size: f32,
    line_height: Option<f32>,
    word_spacing: Option<f32>,
    letter_spacing: Option<f32>,
    alignment: Alignment,
    font_weight: FontWeight, // Axis: WGHT
    font_style: FontStyle,   // Axis: ITAL
    var_axes: Vec<FontVariation>,

    to: usize,
    id: NodeId,
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
