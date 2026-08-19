//! Text-field geometry shared by the painter (drawing the value and caret) and the engine
//! (placing the caret from a click): which slice of the value is visible, and where a char index
//! sits in it. Both sides must agree, so it lives in one place.

use crate::common::font::{FontAlignment, FontInfo};
use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{FontStretch, FontSystem, FontWeight, ShapedText, TextAlign, TextStyle};

const UNBOUNDED: f64 = 1_000_000_000.0;

/// Horizontal inset of the text inside a field's content box.
pub fn inset_x(content_width: f64) -> f64 {
    2.0_f64.min(content_width / 2.0)
}

/// The [`TextStyle`] the layouter measured with (start-aligned; wraps at `max_width`).
pub fn text_style(font_info: &FontInfo, max_width: f64) -> TextStyle {
    let align = match font_info.alignment {
        FontAlignment::Start => TextAlign::Start,
        FontAlignment::Center => TextAlign::Center,
        FontAlignment::End => TextAlign::End,
        FontAlignment::Justify => TextAlign::Justify,
    };
    TextStyle {
        family: font_info.family.clone(),
        size: font_info.size as f32,
        weight: FontWeight(font_info.weight.clamp(1, 1000) as u16),
        style: if font_info.slant != 0 {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        stretch: FontStretch::NORMAL,
        line_height: Some(font_info.line_height as f32),
        letter_spacing: font_info.letter_spacing as f32,
        max_width: Some(max_width.max(1.0) as f32),
        align,
        display_scale: 1.0,
    }
}

fn shape(fs: &mut dyn FontSystem, text: &str, font_info: &FontInfo, max_width: f64) -> ShapedText {
    if text.is_empty() || font_info.size <= 0.0 {
        return ShapedText::empty();
    }
    fs.shape(text, &text_style(font_info, max_width))
}

/// Advance width of `text` on one unbounded line.
pub fn width(fs: &mut dyn FontSystem, text: &str, font_info: &FontInfo) -> f64 {
    shape(fs, text, font_info, UNBOUNDED).width as f64
}

/// (x, y) of the caret at char index `caret`, relative to the text box. Row = lines the shaped
/// prefix occupies (honours soft wraps); x = width of the prefix's last hard line, measured alone.
/// Run x offsets aren't used: Pango reports them relative to the paragraph, not the box.
pub fn caret_offset(fs: &mut dyn FontSystem, text: &str, caret: usize, font_info: &FontInfo, avail: f64) -> (f64, f64) {
    let prefix: String = text.chars().take(caret).collect();
    let line_h = font_info.line_height.max(font_info.size);
    if prefix.is_empty() {
        return (0.0, 0.0);
    }
    let rows = shape(fs, &prefix, font_info, avail)
        .runs
        .iter()
        .map(|r| r.baseline.to_bits())
        .collect::<std::collections::HashSet<_>>()
        .len()
        .max(1);
    // A trailing newline starts an empty row the shaper doesn't report.
    let (row, last_line) = match prefix.rsplit_once('\n') {
        Some((_, "")) => (rows, ""),
        Some((_, tail)) => (rows - 1, tail),
        None => (rows - 1, prefix.as_str()),
    };
    let x = if last_line.is_empty() {
        0.0
    } else {
        let w = width(fs, last_line, font_info);
        // Trailing spaces shape to no advance.
        let trailing = last_line.chars().rev().take_while(|c| *c == ' ').count();
        (w + trailing as f64 * font_info.size * 0.3).min(avail)
    };
    (x, row as f64 * line_h)
}

/// First char to draw so that `text[start..caret]` fits in `width` (single-line scrolling).
pub fn scroll_start_for_caret(
    fs: &mut dyn FontSystem,
    text: &str,
    caret: usize,
    font_info: &FontInfo,
    w: f64,
) -> usize {
    let (mut lo, mut hi) = (0usize, caret);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let seg: String = text.chars().skip(mid).take(caret - mid).collect();
        if width(fs, &seg, font_info) <= w - 1.0 {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// Last char index (exclusive) such that `text[start..end]` fits in `width`.
pub fn fit_end(fs: &mut dyn FontSystem, text: &str, start: usize, font_info: &FontInfo, w: f64) -> usize {
    let total = text.chars().count();
    let (mut lo, mut hi) = (start, total);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let seg: String = text.chars().skip(start).take(mid - start).collect();
        if width(fs, &seg, font_info) <= w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// The visible `[start, end)` char window of a single-line field: the head of the text, or, when
/// the caret would fall past the right edge, scrolled so the caret is inside.
pub fn single_line_window(
    fs: &mut dyn FontSystem,
    text: &str,
    caret: Option<usize>,
    font_info: &FontInfo,
    w: f64,
) -> (usize, usize) {
    let start = match caret {
        Some(c) if caret_offset(fs, text, c, font_info, UNBOUNDED).0 > w => {
            scroll_start_for_caret(fs, text, c, font_info, w)
        }
        _ => 0,
    };
    (start, fit_end(fs, text, start, font_info, w))
}

/// One visual row of a textarea: the char range `[start, end)` it shows (a trailing `\n` is
/// not part of it) and whether it ends its hard line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
    pub hard_end: bool,
}

/// Break `text` into visual rows no wider than `w`: hard lines at `\n`, greedy word wrap inside
/// them (a word that doesn't fit on its own is split by character). Rows are what the painter
/// draws and what a click maps against, so soft wraps are consistent everywhere.
pub fn layout_rows(fs: &mut dyn FontSystem, text: &str, font_info: &FontInfo, w: f64) -> Vec<Row> {
    let mut rows = Vec::new();
    let space_w = width(fs, "a a", font_info) - width(fs, "aa", font_info);
    let mut line_start = 0usize;
    for line in text.split('\n') {
        let line_len = line.chars().count();
        let mut row_start = line_start;
        let mut row_text = String::new();
        // Estimated row width from per-word measures (+ space advances); the exact kerned width
        // is only checked when the estimate says the row is full, so a paragraph costs O(words).
        let mut row_w = 0.0;
        for word in line.split_inclusive(' ') {
            let core = word.trim_end();
            let core_w = width(fs, core, font_info);
            let spaces = (word.chars().count() - core.chars().count()) as f64 * space_w;
            let fits = row_text.is_empty() || row_w + core_w <= w || {
                let candidate = format!("{row_text}{core}");
                width(fs, &candidate, font_info) <= w
            };
            if fits {
                row_text.push_str(word);
                row_w += core_w + spaces;
            } else {
                rows.push(Row {
                    start: row_start,
                    end: row_start + row_text.chars().count(),
                    hard_end: false,
                });
                row_start += row_text.chars().count();
                row_text = word.to_string();
                row_w = core_w + spaces;
            }
            // A single word wider than the row: split it by characters. (Estimate first so the
            // exact measure only runs when a row really overflows.)
            while row_w > w && row_text.chars().count() > 1 && width(fs, row_text.trim_end(), font_info) > w {
                let keep = fit_end(fs, &row_text, 0, font_info, w).max(1);
                rows.push(Row {
                    start: row_start,
                    end: row_start + keep,
                    hard_end: false,
                });
                row_start += keep;
                row_text = row_text.chars().skip(keep).collect();
                row_w = width(fs, row_text.trim_end(), font_info);
            }
        }
        rows.push(Row {
            start: row_start,
            end: line_start + line_len,
            hard_end: true,
        });
        line_start += line_len + 1;
    }
    rows
}

/// The row the caret at char `caret` sits on: a caret at a soft-wrap boundary belongs to the
/// next row (like browsers), at a hard-line end to that row.
pub fn row_of_caret(rows: &[Row], caret: usize) -> usize {
    rows.iter()
        .position(|r| caret < r.end || (caret == r.end && r.hard_end))
        .unwrap_or(rows.len().saturating_sub(1))
}

/// First visible row so that `rows_fit` rows show `caret_row`.
pub fn first_visible_row(total: usize, caret_row: usize, rows_fit: usize) -> usize {
    let rows_fit = rows_fit.max(1);
    if total <= rows_fit {
        return 0;
    }
    caret_row.saturating_sub(rows_fit - 1).min(total - rows_fit)
}

/// Horizontal caret offset within a row's text (`trailing spaces` advance a little so the caret
/// keeps moving past them).
pub fn x_in_row(fs: &mut dyn FontSystem, row_text: &str, chars: usize, font_info: &FontInfo) -> f64 {
    let prefix: String = row_text.chars().take(chars).collect();
    if prefix.is_empty() {
        return 0.0;
    }
    let trailing = prefix.chars().rev().take_while(|c| *c == ' ').count();
    width(fs, &prefix, font_info) + trailing as f64 * font_info.size * 0.3
}

/// `row.text` for `text`.
pub fn row_text(text: &str, row: &Row) -> String {
    text.chars().skip(row.start).take(row.end - row.start).collect()
}

/// The char index in a single line of `text` nearest to horizontal offset `x` (from the line's
/// start). Boundaries are compared by prefix width; the nearer side wins.
pub fn index_at_x(fs: &mut dyn FontSystem, text: &str, font_info: &FontInfo, x: f64) -> usize {
    let n = text.chars().count();
    if n == 0 || x <= 0.0 {
        return 0;
    }
    let mut prefix_w = |i: usize| -> f64 {
        let p: String = text.chars().take(i).collect();
        width(fs, &p, font_info)
    };
    if x >= prefix_w(n) {
        return n;
    }
    // Largest i with width(prefix i) <= x.
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if prefix_w(mid) <= x {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let (w_lo, w_hi) = (prefix_w(lo), prefix_w(lo + 1));
    if x - w_lo > (w_hi - w_lo) / 2.0 {
        lo + 1
    } else {
        lo
    }
}
