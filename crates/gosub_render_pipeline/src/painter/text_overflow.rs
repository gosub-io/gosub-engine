//! `text-overflow: ellipsis` (css-overflow-3 §5.1): inline content that runs past the end edge of
//! a block that does not let it show is cut there, and the cut is marked with "…".
//!
//! Paint commands cannot clip, so the text is cut instead, as a text field's is: the run that
//! crosses the edge keeps the graphemes that fit beside the ellipsis, and a run wholly past the
//! edge is not painted. Each run is decided on its own, so a line whose last visible run happens
//! to end exactly at the edge gets no ellipsis, and the `<string>` form is not supported.

use unicode_segmentation::UnicodeSegmentation;

use crate::layouter::{LayoutElementNode, LayoutTree};
use gosub_interface::style::{Display, Overflow, TextOverflow};

/// The ellipsis, U+2026.
pub const ELLIPSIS: &str = "\u{2026}";

/// What becomes of one run of text.
#[derive(Debug, PartialEq, Eq)]
pub enum Cut {
    /// Painted as it is.
    Whole,
    /// Wholly past the edge: not painted.
    Hidden,
    /// Crosses the edge: painted as this, which ends in the ellipsis.
    Ellipsis(String),
}

/// The end edge, in page px, that the block holding `element`'s line boxes cuts its text at, or
/// `None` when that block does not ask for an ellipsis.
pub fn ellipsis_edge(layout_tree: &LayoutTree, element: &LayoutElementNode) -> Option<f64> {
    let doc = &layout_tree.render_tree.doc;
    // Up through the inline boxes the run is in, to the block whose line boxes hold it. An inline
    // box that is a flex or grid item is blockified, so it is that block.
    let mut id = element.parent?;
    let block = loop {
        let el = layout_tree.arena.get(&id)?;
        let parent = el.parent.and_then(|parent| layout_tree.arena.get(&parent));
        let inline_box = doc.computed_style(el.dom_node_id).box_group.display == Display::Inline
            && !parent.is_some_and(|parent| {
                matches!(
                    doc.computed_style(parent.dom_node_id).box_group.display,
                    Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid
                )
            });
        if !inline_box {
            break el;
        }
        id = el.parent?;
    };
    let style = doc.computed_style(block.dom_node_id);
    // Only a box whose overflow is not shown can mark it (§5.1 "applies to block containers").
    if style.box_group.text_overflow != TextOverflow::Ellipsis || style.box_group.overflow_x == Overflow::Visible {
        return None;
    }
    let content = block.box_model.content_box;
    Some(content.x + content.width)
}

/// Cut `text`, laid out from `x` to `x + width`, at `edge`. `measure` gives the advance of a
/// string in the run's font.
pub fn cut(text: &str, x: f64, width: f64, edge: f64, measure: impl Fn(&str) -> f64) -> Cut {
    // Half a pixel of slack: layout widths are rounded up to the pixel.
    if x + width <= edge + 0.5 {
        return Cut::Whole;
    }
    if x >= edge {
        return Cut::Hidden;
    }
    let room = edge - x - measure(ELLIPSIS);
    // The advance only grows as graphemes are added, so the longest prefix that fits is found by
    // bisecting over the grapheme boundaries.
    let boundaries: Vec<usize> = text.grapheme_indices(true).map(|(at, _)| at).skip(1).collect();
    let (mut fits, mut over) = (0, boundaries.len());
    while fits < over {
        let mid = (fits + over).div_ceil(2);
        if measure(&text[..boundaries[mid - 1]]) <= room {
            fits = mid;
        } else {
            over = mid - 1;
        }
    }
    let kept = if fits == 0 { "" } else { &text[..boundaries[fits - 1]] };
    // A space before the ellipsis is not kept: "cut …" reads as a gap, not a cut.
    Cut::Ellipsis(format!("{}{ELLIPSIS}", kept.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character ten pixels wide.
    fn measure(text: &str) -> f64 {
        10.0 * text.chars().count() as f64
    }

    #[test]
    fn a_run_inside_the_edge_is_left_whole() {
        assert_eq!(cut("abc", 0.0, 30.0, 30.0, measure), Cut::Whole);
        assert_eq!(cut("abc", 100.0, 30.0, 200.0, measure), Cut::Whole);
    }

    #[test]
    fn a_run_past_the_edge_is_hidden() {
        assert_eq!(cut("abc", 50.0, 30.0, 50.0, measure), Cut::Hidden);
    }

    #[test]
    fn a_run_across_the_edge_keeps_what_fits_beside_the_ellipsis() {
        // 55px of room, 10px of it for the ellipsis: four letters fit.
        assert_eq!(
            cut("abcdefghij", 0.0, 100.0, 55.0, measure),
            Cut::Ellipsis("abcd\u{2026}".into())
        );
        // Not even one letter fits beside it: the ellipsis alone.
        assert_eq!(
            cut("abcdefghij", 0.0, 100.0, 15.0, measure),
            Cut::Ellipsis("\u{2026}".into())
        );
        // The space the cut lands after is dropped.
        assert_eq!(
            cut("ab cdefgh", 0.0, 90.0, 45.0, measure),
            Cut::Ellipsis("ab\u{2026}".into())
        );
    }

    #[test]
    fn a_cut_never_splits_a_grapheme() {
        // "e" and a combining acute are one grapheme, two chars: twenty pixels here. There is room
        // for "ab" and one char of it, which is not enough for the grapheme.
        assert_eq!(
            cut("abe\u{301}cd", 0.0, 60.0, 40.0, measure),
            Cut::Ellipsis("ab\u{2026}".into())
        );
    }
}
