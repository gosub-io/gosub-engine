//! Inline-run collection — Stage 0 of the inline-formatting rework.
//! See `docs/inline-run-rework-plan.md`.
//!
//! Collects a block's inline content (text nodes + flattened inline elements) into a flat list of
//! styled [`InlineSegment`]s, split into line boxes at `<br>`, with CSS `white-space: normal`
//! collapsing applied across segment boundaries and `text-transform` applied per segment.
//!
//! This is scaffolding: it is **not yet wired into layout** (Stage 1 replaces the flex-based
//! anonymous line container with a single run leaf built from this). Until then the doc-walking
//! entry points are unused, hence the module-wide `dead_code` allow — it comes off in Stage 1. The
//! whitespace-collapsing and line-box-splitting logic is pure and unit-tested below.
#![allow(dead_code)]

use std::sync::Arc;

use crate::common::document::node::NodeType;
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::document::style::{lookup, FontWeight, StyleProperty, Unit, Value};
use gosub_shared::node::NodeId;

const DEFAULT_FONT_SIZE: f32 = 16.0;
const DEFAULT_FONT_FAMILY: &str = "sans-serif";

/// CSS `text-transform` applied to a segment's text after whitespace collapsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

/// Resolved text styling shared by one contiguous run of characters.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentStyle {
    pub font_family: String,
    pub font_size: f32,
    pub weight: i32,
    pub italic: bool,
    pub color: (u8, u8, u8, u8),
    pub underline: bool,
    pub line_through: bool,
    pub line_height: f32,
}

/// One run of text sharing a single style, tagged with the DOM node it came from (used later for
/// hit-testing, per-span backgrounds, and box-model rects).
#[derive(Debug, Clone, PartialEq)]
pub struct InlineSegment {
    pub text: String,
    pub style: SegmentStyle,
    pub source: NodeId,
}

/// A text chunk as walked from the tree — still holding source whitespace, pre-collapse.
#[derive(Debug, Clone, PartialEq)]
struct TextEntry {
    text: String,
    style: SegmentStyle,
    source: NodeId,
    transform: TextTransform,
}

/// Flat walk output before line-box splitting / whitespace collapsing.
#[derive(Debug, Clone, PartialEq)]
enum RawEntry {
    Text(TextEntry),
    /// A forced line break (`<br>`), carrying the line-height for a blank line it may produce.
    Break(f32),
}

/// Collect the inline content directly under `block` into styled segments, split into line boxes at
/// `<br>`. The outer `Vec` is one entry per line box; an empty inner `Vec` is a blank line (a
/// standalone `<br>`).
pub fn collect_inline_run(doc: &Arc<dyn PipelineDocument>, block: NodeId) -> Vec<Vec<InlineSegment>> {
    let mut raw = Vec::new();
    for child in doc.children(block) {
        walk(doc, child, &mut raw);
    }
    split_line_boxes(raw).into_iter().map(finish_box).collect()
}

/// Recursively append `node_id`'s inline content to `out`. Inline elements are flattened (their
/// children join the parent run); `<br>` emits a break; block / inline-block children are not part
/// of an inline run and are skipped here (Stage 6 handles inline-block as an inline box).
fn walk(doc: &Arc<dyn PipelineDocument>, node_id: NodeId, out: &mut Vec<RawEntry>) {
    let Some(node) = doc.get_node_by_id(node_id) else {
        return;
    };

    match &node.node_type {
        NodeType::Comment(_) => {}
        NodeType::Text(text) => {
            out.push(RawEntry::Text(TextEntry {
                text: text.clone(),
                style: resolve_segment_style(doc, node_id),
                source: node_id,
                transform: resolve_text_transform(doc, node_id),
            }));
        }
        NodeType::Element(data) => {
            if data.tag_name.eq_ignore_ascii_case("br") {
                let fs = font_size_of(doc, node_id);
                out.push(RawEntry::Break(resolve_line_height(doc, node_id, fs)));
                return;
            }
            if node.is_inline_element() {
                for child in doc.children(node_id) {
                    walk(doc, child, out);
                }
            }
        }
    }
}

fn font_size_of(doc: &Arc<dyn PipelineDocument>, id: NodeId) -> f32 {
    match doc.get_style(id, &StyleProperty::FontSize) {
        Value::Unit(v, Unit::Px) => v,
        _ => DEFAULT_FONT_SIZE,
    }
}

fn resolve_line_height(doc: &Arc<dyn PipelineDocument>, id: NodeId, font_size: f32) -> f32 {
    match doc.get_style(id, &StyleProperty::LineHeight) {
        Value::Unit(v, Unit::Px) => v,
        Value::Number(ratio) => font_size * ratio,
        _ => font_size * 1.4,
    }
}

fn resolve_text_transform(doc: &Arc<dyn PipelineDocument>, id: NodeId) -> TextTransform {
    match doc.get_style(id, &StyleProperty::TextTransform) {
        Value::Keyword(kw) => match lookup(kw).as_str() {
            "uppercase" => TextTransform::Uppercase,
            "lowercase" => TextTransform::Lowercase,
            "capitalize" => TextTransform::Capitalize,
            _ => TextTransform::None,
        },
        _ => TextTransform::None,
    }
}

/// Resolve a node's computed inline styling. Text nodes have no own style, but `get_style` walks the
/// parent chain for inherited properties, so calling this on a text node returns the styling it
/// inherits from its inline/block ancestors.
fn resolve_segment_style(doc: &Arc<dyn PipelineDocument>, id: NodeId) -> SegmentStyle {
    let font_size = font_size_of(doc, id);

    let font_family = match doc.get_style(id, &StyleProperty::FontFamily) {
        Value::Keyword(kw) => lookup(kw),
        _ => DEFAULT_FONT_FAMILY.to_string(),
    };

    let weight = match doc.get_style(id, &StyleProperty::FontWeight) {
        Value::FontWeight(w) => match w {
            FontWeight::Normal => 400,
            FontWeight::Bold | FontWeight::Bolder => 700,
            FontWeight::Lighter => 300,
            FontWeight::Number(v) => v as i32,
        },
        _ => 400,
    };

    let italic = matches!(
        doc.get_style(id, &StyleProperty::FontStyle),
        Value::Keyword(kw) if lookup(kw) == "italic"
    );

    let color = match doc.get_style(id, &StyleProperty::Color) {
        Value::Color(r, g, b, a) => (r, g, b, a),
        _ => (0, 0, 0, 255),
    };

    let decoration = match doc.get_style(id, &StyleProperty::TextDecorationLine) {
        Value::Keyword(kw) => lookup(kw),
        _ => String::new(),
    };

    SegmentStyle {
        font_family,
        font_size,
        weight,
        italic,
        color,
        underline: decoration.contains("underline"),
        line_through: decoration.contains("line-through"),
        line_height: resolve_line_height(doc, id, font_size),
    }
}

// ── Pure post-processing (unit-tested) ──────────────────────────────────────────

/// Split a flat run into line boxes at `<br>`. A final box is always produced (possibly empty),
/// and a break with no following content yields a trailing empty box (a blank line).
fn split_line_boxes(entries: Vec<RawEntry>) -> Vec<Vec<TextEntry>> {
    let mut boxes = Vec::new();
    let mut current = Vec::new();
    for entry in entries {
        match entry {
            RawEntry::Text(t) => current.push(t),
            RawEntry::Break(_) => boxes.push(std::mem::take(&mut current)),
        }
    }
    boxes.push(current);
    boxes
}

/// Apply CSS `white-space: normal` collapsing to one line box and emit its segments.
///
/// Runs of ASCII whitespace collapse to a single space, including across segment boundaries; the
/// box's leading and trailing whitespace is trimmed. A whitespace-only segment (e.g. the text node
/// between two inline elements) contributes the collapsed inter-element space to its neighbour
/// rather than a segment of its own. `text-transform` is applied per segment after collapsing.
/// NBSP (U+00A0) is intentionally not collapsed.
fn finish_box(entries: Vec<TextEntry>) -> Vec<InlineSegment> {
    let mut out = Vec::new();
    // A collapsed space waiting to be emitted before the next non-space character; carries across
    // segment boundaries so inter-element whitespace survives as a single space.
    let mut pending_space = false;
    // Have we emitted any non-space yet in this box (drives leading-whitespace trimming)?
    let mut seen_non_space = false;

    for entry in entries {
        let mut seg = String::new();
        for ch in entry.text.chars() {
            if ch.is_ascii_whitespace() {
                if seen_non_space {
                    pending_space = true;
                }
                // Otherwise this is leading whitespace of the box — drop it.
            } else {
                if pending_space {
                    seg.push(' ');
                    pending_space = false;
                }
                seg.push(ch);
                seen_non_space = true;
            }
        }

        let seg = apply_transform(&seg, entry.transform);
        if !seg.is_empty() {
            out.push(InlineSegment {
                text: seg,
                style: entry.style,
                source: entry.source,
            });
        }
    }
    // A trailing `pending_space` is never flushed → trailing whitespace is trimmed.
    out
}

fn apply_transform(text: &str, transform: TextTransform) -> String {
    match transform {
        TextTransform::None => text.to_string(),
        TextTransform::Uppercase => text.to_uppercase(),
        TextTransform::Lowercase => text.to_lowercase(),
        TextTransform::Capitalize => {
            // First letter of each whitespace-separated word. Word boundaries are not tracked across
            // segment boundaries (a word split by an inline element is a Stage-3+ concern).
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> SegmentStyle {
        SegmentStyle {
            font_family: "serif".into(),
            font_size: 16.0,
            weight: 400,
            italic: false,
            color: (0, 0, 0, 255),
            underline: false,
            line_through: false,
            line_height: 24.0,
        }
    }

    fn te(text: &str, src: usize) -> TextEntry {
        TextEntry {
            text: text.into(),
            style: style(),
            source: NodeId::from(src),
            transform: TextTransform::None,
        }
    }

    fn te_tf(text: &str, tf: TextTransform) -> TextEntry {
        TextEntry {
            text: text.into(),
            style: style(),
            source: NodeId::from(1usize),
            transform: tf,
        }
    }

    fn joined(segs: &[InlineSegment]) -> String {
        segs.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn collapses_internal_and_trims_edge_whitespace() {
        let segs = finish_box(vec![te("  The   web  should be  ", 1)]);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "The web should be");
    }

    #[test]
    fn preserves_single_space_across_segments() {
        // "The web should be " + <em>"open and free"</em> + " for everyone"
        let segs = finish_box(vec![
            te("The web should be ", 1),
            te("open and free", 2),
            te(" for everyone", 3),
        ]);
        assert_eq!(joined(&segs), "The web should be open and free for everyone");
        assert_eq!(segs.len(), 3);
        // The middle (em) segment keeps its own source node for later styling/hit-testing.
        assert_eq!(segs[1].source, NodeId::from(2usize));
    }

    #[test]
    fn whitespace_only_segment_becomes_single_inter_element_space() {
        // "<span>A</span> <span>B</span>" — the " " text node between the spans.
        let segs = finish_box(vec![te("A", 1), te(" ", 2), te("B", 3)]);
        assert_eq!(joined(&segs), "A B");
        // The whitespace-only node produces no segment of its own.
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn trims_box_leading_and_trailing_whitespace_only_segments() {
        let segs = finish_box(vec![te("   ", 1), te("hello", 2), te("   ", 3)]);
        assert_eq!(joined(&segs), "hello");
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn splits_line_boxes_at_breaks() {
        let raw = vec![
            RawEntry::Text(te("a", 1)),
            RawEntry::Break(20.0),
            RawEntry::Text(te("b", 2)),
            RawEntry::Text(te("c", 3)),
        ];
        let boxes = split_line_boxes(raw);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].len(), 1);
        assert_eq!(boxes[1].len(), 2);
    }

    #[test]
    fn trailing_break_produces_empty_box() {
        let raw = vec![RawEntry::Text(te("a", 1)), RawEntry::Break(20.0)];
        let boxes = split_line_boxes(raw);
        assert_eq!(boxes.len(), 2);
        assert!(boxes[1].is_empty());
    }

    #[test]
    fn applies_text_transform_per_segment() {
        assert_eq!(finish_box(vec![te_tf("working", TextTransform::Uppercase)])[0].text, "WORKING");
        assert_eq!(finish_box(vec![te_tf("WORKING", TextTransform::Lowercase)])[0].text, "working");
        assert_eq!(
            finish_box(vec![te_tf("early stage", TextTransform::Capitalize)])[0].text,
            "Early Stage"
        );
    }

    #[test]
    fn copyright_case_splits_and_orders_correctly() {
        // <p>Copyright …, the Gosub community.<br>Spotted an issue? <a>Send a PR on GitHub</a>.</p>
        let raw = vec![
            RawEntry::Text(te("Copyright 2024\u{2013}2026, the Gosub community.", 1)),
            RawEntry::Break(20.0),
            RawEntry::Text(te("Spotted an issue? ", 2)),
            RawEntry::Text(te("Send a PR on GitHub", 3)), // the <a> link
            RawEntry::Text(te(".", 4)),
        ];
        let boxes: Vec<Vec<InlineSegment>> = split_line_boxes(raw).into_iter().map(finish_box).collect();
        assert_eq!(boxes.len(), 2);
        assert_eq!(joined(&boxes[0]), "Copyright 2024\u{2013}2026, the Gosub community.");
        assert_eq!(joined(&boxes[1]), "Spotted an issue? Send a PR on GitHub.");
        // The link keeps its own source node so it can later be coloured/underlined/hit-tested.
        assert!(boxes[1].iter().any(|s| s.source == NodeId::from(3usize)));
    }
}
