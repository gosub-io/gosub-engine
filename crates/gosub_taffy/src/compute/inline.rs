use parley::fontique::{FallbackKey, FontWeight, Script};
use parley::{AlignmentOptions, FontContext};
use std::sync::{LazyLock, Mutex};
use taffy::{
    AvailableSpace, CollapsibleMarginSet, Layout, LayoutInput, LayoutOutput, LayoutPartialTree, NodeId, Point, Rect,
    RunMode, Size,
};

use gosub_interface::config::HasLayouter;
use gosub_interface::css3::{CssProperty, CssValue};
use gosub_interface::font::{FontBlob, FontInfo, FontManager, FontStyle, HasFontManager};
use gosub_interface::layout::{Decoration, DecorationStyle, HasTextLayout, LayoutNode, LayoutTree};
use gosub_shared::font::Glyph;
use gosub_shared::geo::FP;
use gosub_shared::{geo, ROBOTO_FONT};

use crate::text::TextLayout;
use crate::{Display, LayoutDocument, TaffyLayouter};

static FONT_CX: LazyLock<Mutex<FontContext>> = LazyLock::new(|| {
    let mut ctx = FontContext::default();

    let fonts = ctx.collection.register_fonts(ROBOTO_FONT.to_vec());

    ctx.collection
        .append_fallbacks(FallbackKey::new(Script::from("Latn"), None), fonts.iter().map(|f| f.0));

    Mutex::new(ctx)
});

/// Computes the layout for inline elements.
pub fn compute_inline_layout<C: HasLayouter<Layouter = TaffyLayouter>>(
    tree: &mut LayoutDocument<C>,
    node_id: <C::LayoutTree as LayoutTree<C>>::NodeId,
    mut layout_input: LayoutInput,
) -> LayoutOutput {
    layout_input.known_dimensions = Size::NONE;
    layout_input.run_mode = RunMode::PerformLayout; //TODO: We should respect the run mode
                                                    // layout_input.sizing_mode = SizingMode::ContentSize;

    // If there are no children, the node is hidden
    let Some(children) = tree.0.children(node_id) else {
        return LayoutOutput::HIDDEN;
    };

    // Either a node is an inline element (for instance, an image aligned or inside a text), or an
    // actual text node.

    // The buffer that holds the text data of the node
    let mut str_buf = String::new();
    // Text node data holds the information about the text nodes. There can be multiple text node data elements for
    // a single text, for instance, if there are different font sizes or weights inside the text.
    let mut text_node_data: Vec<TextNodeData<C>> = Vec::new();
    // List of any inline boxes that are inside the node
    let mut inline_boxes = Vec::new();

    // Generate the text data and inline boxes. A text can consist of multiple text nodes, for instance, if there are
    // different font sizes or weights inside the text. For instance:  "This is a <b>bold</b> text". In this example
    // there will be three text nodes: "This is a ", "bold" and " text". The first and last will have no bold font,
    // but the second has a bold font.
    for child in &children {
        let child_node_id = NodeId::from((*child).into());

        // If the child is not a node, we skip it
        let Some(node) = tree.0.get_node_mut(*child) else {
            continue;
        };
        node.clear_text_layout();

        if let Some(text) = node.text_data() {
            // We found a text node
            if text.is_empty() {
                continue;
            }

            // Empty or whitespace only text nodes are ignored
            let only_whitespace = text.chars().all(|c| c.is_whitespace());
            if only_whitespace {
                continue;
            }

            // We add a space between the text nodes, so that the text is not glued together
            str_buf.push(' ');
            str_buf.push_str(text);

            // @TODO: default font family can be different per platform
            let font_family = node
                .get_property("font-family")
                .and_then(|s| s.as_string())
                .map(|s| s.to_string())
                .unwrap_or("sans-serif".to_string());

            let font_size = node.get_property("font-size").map(|s| s.unit_to_px()).unwrap_or(16.0);
            let alignment = parse_alignment(node);
            let font_weight = parse_font_weight(node);
            let font_style = parse_font_style(node);
            let var_axes = parse_font_axes(node);
            let line_height = node.get_property("line-height").and_then(|s| s.as_number());
            let word_spacing = node.get_property("word-spacing").map(|s| s.unit_to_px());
            let letter_spacing = node.get_property("letter-spacing").map(|s| s.unit_to_px());

            let font_info = <C::FontManager as FontManager>::FontInfo::new(&font_family)
                .unwrap()
                .with_weight(font_weight.value() as i32)
                .with_style(font_style);

            let mut underline = false;
            let mut overline = false;
            let mut line_through = false;
            let mut decoration_width = 1.0;
            let mut decoration_color = (0.0, 0.0, 0.0, 1.0);
            let mut style = DecorationStyle::Solid;
            let mut underline_offset = 4.0;
            let color = node.get_property("color").and_then(|s| s.parse_color());

            // Generate decoration styles
            if let Some(actual_parent) = tree.0.parent_id(node_id) {
                if let Some(node) = tree.0.get_node_mut(actual_parent) {
                    let decoration_line = node.get_property("text-decoration-line");

                    if let Some(decoration_line) = decoration_line {
                        if let Some(list) = decoration_line.as_list() {
                            for item in list {
                                if let Some(s) = item.as_string() {
                                    match s {
                                        "underline" => underline = true,
                                        "overline" => overline = true,
                                        "line-through" => line_through = true,
                                        _ => {}
                                    }
                                }
                            }
                        } else if let Some(s) = decoration_line.as_string() {
                            match s {
                                "underline" => underline = true,
                                "overline" => overline = true,
                                "line-through" => line_through = true,
                                _ => {}
                            }
                        }
                    }

                    let decoration_style = node.get_property("text-decoration-style");

                    match decoration_style.as_ref().map(|s| s.as_string()) {
                        Some(Some("solid")) => style = DecorationStyle::Solid,
                        Some(Some("double")) => style = DecorationStyle::Double,
                        Some(Some("dotted")) => style = DecorationStyle::Dotted,
                        Some(Some("dashed")) => style = DecorationStyle::Dashed,
                        Some(Some("wavy")) => style = DecorationStyle::Wavy,
                        _ => {}
                    }

                    decoration_width = node
                        .get_property("text-decoration-thickness")
                        .map(|s| s.unit_to_px())
                        .unwrap_or(1.0);

                    if let Some(c) = node
                        .get_property("text-decoration-color")
                        .and_then(|s| s.parse_color())
                        .or(color)
                    {
                        decoration_color = c;
                    }

                    if let Some(o) = node.get_property("text-underline-offset").map(|s| s.unit_to_px()) {
                        underline_offset = o;
                    }
                }
            }

            text_node_data.push(TextNodeData {
                font_info,
                font_size,
                line_height,
                word_spacing,
                letter_spacing,
                alignment,
                var_axes,

                decoration: Decoration {
                    underline,
                    overline,
                    line_through,
                    color: decoration_color,
                    style,
                    width: decoration_width,
                    underline_offset,
                    x_offset: 0.0,
                },

                to: str_buf.len(),
                id: child_node_id,
            });
        } else {
            // We found an inline box
            let out = tree.compute_child_layout(child_node_id, layout_input);

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

            inline_boxes.push(parley::InlineBox {
                id: child_node_id.into(),
                index: str_buf.len(),
                height: size.height,
                width: size.width,
            });
        }
    }

    // No inline boxes or text data, so the node is hidden
    if inline_boxes.is_empty() && str_buf.is_empty() {
        return LayoutOutput::HIDDEN;
    }

    // We we don't have a text node, we add an empty character to the buffer
    if str_buf.is_empty() {
        str_buf.push(0 as char);
    }

    // We use the parley layout engine to generate the text layout
    let mut layout_cx: parley::LayoutContext<usize> = parley::LayoutContext::new();
    // let mut scale_cx = ScaleContext::new();

    let mut font_context = FONT_CX.lock().unwrap();

    let mut builder = layout_cx.ranged_builder(&mut font_context, &str_buf, 1.0);
    let mut align = parley::Alignment::default();

    // The first text node is the default style for the text. This is why this is treated separately.
    if let Some(default) = text_node_data.first() {
        let info: &<<C as HasFontManager>::FontManager as FontManager>::FontInfo = default.font_info();

        builder.push_default(parley::StyleProperty::FontStack(parley::FontStack::Single(
            parley::FontFamily::Named(info.family().into()),
        )));
        builder.push_default(parley::StyleProperty::FontSize(default.font_size));
        if let Some(line_height) = default.line_height {
            builder.push_default(parley::StyleProperty::LineHeight(line_height));
        }
        if let Some(word_spacing) = default.word_spacing {
            builder.push_default(parley::StyleProperty::WordSpacing(word_spacing));
        }
        if let Some(letter_spacing) = default.letter_spacing {
            builder.push_default(parley::StyleProperty::LetterSpacing(letter_spacing));
        }
        builder.push_default(parley::StyleProperty::FontWeight(FontWeight::new(info.weight() as f32)));
        builder.push_default(parley::StyleProperty::FontStyle(match info.style() {
            FontStyle::Normal => parley::FontStyle::Normal,
            FontStyle::Italic => parley::FontStyle::Italic,
            FontStyle::Oblique => parley::FontStyle::Oblique(None),
        }));
        builder.push_default(parley::StyleProperty::FontVariations(parley::FontSettings::List(
            default.var_axes.as_slice().into(),
        )));

        if default.decoration.overline && default.decoration.underline {
            builder.push_default(parley::StyleProperty::Underline(true));

            builder.push_default(parley::StyleProperty::UnderlineSize(Some(
                default.decoration.width * 2.0,
            )));
            builder.push_default(parley::StyleProperty::UnderlineOffset(Some(
                default.decoration.underline_offset,
            )));
        } else if default.decoration.overline {
            builder.push_default(parley::StyleProperty::Underline(true));

            builder.push_default(parley::StyleProperty::UnderlineSize(Some(default.decoration.width)));
        } else if default.decoration.underline {
            builder.push_default(parley::StyleProperty::Underline(true));

            builder.push_default(parley::StyleProperty::UnderlineSize(Some(default.decoration.width)));
            builder.push_default(parley::StyleProperty::UnderlineOffset(Some(
                default.decoration.underline_offset,
            )));
        }

        builder.push_default(parley::StyleProperty::Brush(0));

        align = default.alignment;

        let mut from = default.to;

        for (idx, text_node) in text_node_data.get(1..).unwrap_or_default().iter().enumerate() {
            let info: &<<C as HasFontManager>::FontManager as FontManager>::FontInfo = text_node.font_info();

            builder.push(
                parley::StyleProperty::FontStack(parley::FontStack::Source(info.family().into())),
                from..text_node.to,
            );
            builder.push(parley::StyleProperty::FontSize(text_node.font_size), from..text_node.to);
            if let Some(line_height) = text_node.line_height {
                builder.push(parley::StyleProperty::LineHeight(line_height), from..text_node.to);
            }
            if let Some(word_spacing) = text_node.word_spacing {
                builder.push(parley::StyleProperty::WordSpacing(word_spacing), from..text_node.to);
            }
            if let Some(letter_spacing) = text_node.letter_spacing {
                builder.push(parley::StyleProperty::LetterSpacing(letter_spacing), from..text_node.to);
            }
            builder.push(
                parley::StyleProperty::FontWeight(FontWeight::new(info.weight() as f32)),
                from..text_node.to,
            );
            builder.push(
                parley::StyleProperty::FontStyle(match info.style() {
                    FontStyle::Normal => parley::FontStyle::Normal,
                    FontStyle::Italic => parley::FontStyle::Italic,
                    FontStyle::Oblique => parley::FontStyle::Oblique(None),
                }),
                from..text_node.to,
            );
            builder.push(
                parley::StyleProperty::FontVariations(parley::FontSettings::List(text_node.var_axes.as_slice().into())),
                from..text_node.to,
            );

            builder.push(parley::StyleProperty::Brush(idx), from..text_node.to);

            if default.decoration.overline && default.decoration.underline {
                builder.push(parley::StyleProperty::Underline(true), from..text_node.to);

                builder.push(
                    parley::StyleProperty::UnderlineSize(Some(default.decoration.width * 2.0)),
                    from..text_node.to,
                );
                builder.push(
                    parley::StyleProperty::UnderlineOffset(Some(default.decoration.underline_offset + 4.0)),
                    from..text_node.to,
                );
            } else if default.decoration.overline {
                builder.push(parley::StyleProperty::Underline(true), from..text_node.to);

                builder.push(
                    parley::StyleProperty::UnderlineSize(Some(default.decoration.width)),
                    from..text_node.to,
                );
                builder.push(parley::StyleProperty::UnderlineOffset(Some(4.0)), from..text_node.to);
            } else if default.decoration.underline {
                builder.push(parley::StyleProperty::Underline(true), from..text_node.to);

                builder.push(
                    parley::StyleProperty::UnderlineSize(Some(default.decoration.width)),
                    from..text_node.to,
                );
                builder.push(
                    parley::StyleProperty::UnderlineOffset(Some(default.decoration.underline_offset)),
                    from..text_node.to,
                );
            }

            builder.push(
                parley::StyleProperty::Underline(default.decoration.underline || default.decoration.overline),
                from..text_node.to,
            );

            from = text_node.to;
        }
    }

    for inline_box in inline_boxes {
        builder.push_inline_box(inline_box);
    }

    let mut layout = builder.build(&str_buf);

    drop(font_context);

    let max_width = match layout_input.available_space.width {
        AvailableSpace::Definite(width) => Some(width),
        AvailableSpace::MinContent => Some(0.0),
        AvailableSpace::MaxContent => None,
    };

    layout.break_all_lines(max_width);

    layout.align(
        None,
        align,
        AlignmentOptions {
            align_when_overflowing: true,
        },
    );

    let content_size = Size {
        width: layout.width().ceil(),
        height: layout.height().ceil(),
    };

    let mut current_node_idx = 0;
    let mut current_node_id = <C::LayoutTree as LayoutTree<C>>::NodeId::from(0);
    let mut current_to = 0;

    if let Some(first) = text_node_data.first() {
        current_node_id = <C::LayoutTree as LayoutTree<C>>::NodeId::from(first.id.into());
        current_to = first.to;
    }

    let mut current_glyph_idx = 0;

    let mut ids = Vec::with_capacity(text_node_data.len());

    'lines: for line in layout.lines() {
        let metrics = line.metrics();

        let height = metrics.line_height;

        for item in line.items() {
            match item {
                parley::PositionedLayoutItem::GlyphRun(run) => {
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
                            current_node_id = <C::LayoutTree as LayoutTree<C>>::NodeId::from(next.id.into());
                        } else {
                            break 'lines;
                        }
                    }

                    let size = geo::Size {
                        width: run.advance(),
                        height,
                    };

                    let coords = grun.normalized_coords().to_owned();

                    let data = text_node_data.get(run.style().brush);

                    let mut decoration = data.map(|x| x.decoration.clone()).unwrap_or_default();

                    if let Some(text) = str_buf.get(grun.text_range()) {
                        let first_non_ws = text.chars().position(|c| !c.is_whitespace());

                        match first_non_ws {
                            None => {}
                            Some(0) => {}

                            Some(i) => {
                                if let Some(g) = glyphs.get(i) {
                                    decoration.x_offset = g.x;
                                }
                            }
                        }
                    }

                    let font = grun.font().clone();
                    let (font_data, _) = font.data.into_raw_parts();

                    let text_layout = TextLayout {
                        glyphs,
                        size,
                        font_size: fs,
                        // Actual font that is resolved by the layouter which is used for these set of glyphs
                        font_data: FontBlob::new(font_data, font.index),
                        coords,
                        decoration,
                        offset: geo::Point {
                            x: run.offset() as FP,
                            y: run_y as FP,
                        },
                    };

                    let node_id = data
                        .map(|x| <C::LayoutTree as LayoutTree<C>>::NodeId::from(x.id.into()))
                        .unwrap_or(current_node_id);

                    ids.push(node_id);

                    let Some(node) = tree.0.get_node_mut(node_id) else {
                        continue;
                    };

                    node.add_text_layout(text_layout);
                }
                parley::PositionedLayoutItem::InlineBox(inline_box) => {
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
                            margin: Rect::ZERO, //TODO: we currently handle margins in the text layout, but we should handle them here
                        },
                    );
                }
            }
        }
    }

    for id in ids {
        let Some(node) = tree.0.get_node_mut(id) else { continue };

        let Some(layouts) = node.get_text_layouts_mut() else {
            continue;
        };

        let mut location = Point {
            x: f32::INFINITY,
            y: f32::INFINITY,
        };
        let mut size = Size::ZERO;

        for layout in &*layouts {
            location.x = location.x.min(layout.offset.x);
            location.y = location.y.min(layout.offset.y - layout.size.height);

            size.width = size.width.max(layout.size.width + layout.offset.x);
            size.height = size.height.max(layout.offset.y);
        }

        for layout in layouts {
            layout.offset.x -= location.x;
            layout.offset.y -= location.y;
        }

        tree.set_unrounded_layout(
            NodeId::new(current_node_id.into()),
            &Layout {
                size,
                content_size: size,
                scrollbar_size: Size::ZERO,
                border: Rect::ZERO,
                location,
                order: 0,
                padding: Rect::ZERO,
                margin: Rect::ZERO,
            },
        );
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

/// Structure that holds information for a (partial) text that consists of a single font size, weight, etc.
/// If a string consists of multiple font sizes, weights, etc., there will be multiple TextNodeData elements.
/// For instance: "This is a <b>bold</b> text". In this example there will be three text nodes: "This is a ",
/// "bold" and " text" with different font weights.
#[derive(Debug)]
struct TextNodeData<C: HasFontManager> {
    /// Start index of the text node in the complete string (str_buf)
    to: usize,
    /// Node identifier that holds the text
    id: NodeId,
    /// Actual font for rendering and layouting
    font_info: <<C as HasFontManager>::FontManager as FontManager>::FontInfo,
    /// Font size
    font_size: f32,
    /// Line height in case of multiple lines
    line_height: Option<f32>,
    /// Spacing between words
    word_spacing: Option<f32>,
    /// Spacing between letters (glyphs?)
    letter_spacing: Option<f32>,
    /// Alignment of the text
    alignment: parley::Alignment,
    /// Unknown
    var_axes: Vec<parley::FontVariation>,
    /// Decoration of the font (strikethrough, underline etc)
    decoration: Decoration,
}

impl<C: HasFontManager> TextNodeData<C> {
    /// Returns the font info of the text node
    pub fn font_info(&self) -> &<<C as HasFontManager>::FontManager as FontManager>::FontInfo {
        &self.font_info
    }
}

fn parse_alignment<C: HasLayouter>(node: &mut impl LayoutNode<C>) -> parley::Alignment {
    let Some(prop) = node.get_property("text-align") else {
        return parley::Alignment::Start;
    };

    let Some(s) = prop.as_string() else {
        return parley::Alignment::Start;
    };

    match s {
        "left" => parley::Alignment::Start,
        "center" => parley::Alignment::Middle,
        "right" => parley::Alignment::End,
        "justify" => parley::Alignment::Justified,
        _ => parley::Alignment::Start,
    }
}

fn parse_font_weight<C: HasLayouter>(node: &mut impl LayoutNode<C>) -> parley::FontWeight {
    let Some(prop) = node.get_property("font-weight") else {
        return parley::FontWeight::NORMAL;
    };

    let Some(s) = prop.as_string() else {
        if let Some(v) = prop.as_number() {
            return parley::FontWeight::new(v);
        };

        return parley::FontWeight::NORMAL;
    };

    match s {
        "thin" => parley::FontWeight::THIN,
        "extra-light" => parley::FontWeight::EXTRA_LIGHT,
        "light" => parley::FontWeight::LIGHT,
        "semi-light" => parley::FontWeight::SEMI_LIGHT,
        "normal" => parley::FontWeight::NORMAL,
        "medium" => parley::FontWeight::MEDIUM,
        "semi-bold" => parley::FontWeight::SEMI_BOLD,
        "bold" => parley::FontWeight::BOLD,
        "extra-bold" => parley::FontWeight::EXTRA_BOLD,
        "black" => parley::FontWeight::BLACK,
        "extra-black" => parley::FontWeight::EXTRA_BLACK,
        _ => parley::FontWeight::NORMAL,
    }
}

fn parse_font_style<C: HasLayouter>(node: &mut impl LayoutNode<C>) -> FontStyle {
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

fn parse_font_axes<C: HasLayouter>(n: &mut impl LayoutNode<C>) -> Vec<parley::FontVariation> {
    let prop = n.get_property("font-variation-settings");

    let Some(s) = prop else {
        return Vec::new();
    };

    // we don't need to care about things other than a list, since you always need two values for a variation
    let Some(mut slice) = s.as_list() else {
        return Vec::new();
    };

    let mut vars = Vec::with_capacity((slice.len() as f32 / 3.0).ceil() as usize);

    loop {
        let Some((candidates, new_slice)) = slice.split_at_checked(2) else {
            break;
        };

        let axis = &candidates[0];

        if axis.is_comma() {
            slice = &slice[1..]; // we can guarantee that this won't panic since we know that we have at least 2 elements because of the first check
            continue;
        }

        let value = &candidates[1];

        slice = new_slice; // we can now update the slice, since no matter if we have a comma or not, we need to move to the next pair

        if value.is_comma() {
            continue;
        }
        let Some(value) = value.as_number() else {
            continue;
        };

        let Some(axis) = axis.as_string() else {
            continue;
        };

        let Ok(tag_bytes): Result<[u8; 4], _> = axis.as_bytes().try_into() else {
            continue;
        };

        let tag = u32::from_be_bytes(tag_bytes);

        vars.push(parley::FontVariation { tag, value });
    }

    vars
}
