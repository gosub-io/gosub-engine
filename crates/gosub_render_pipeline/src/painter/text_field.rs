//! Text-field geometry shared by the painter (drawing the value and caret) and the engine
//! (placing the caret from a click): which slice of the value is visible, and where a char index
//! sits in it. Both sides must agree, so it lives in one place.

use crate::common::font::{FontAlignment, FontInfo};
use crate::common::geo::Rect;
use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{FontStretch, FontSystem, FontWeight, ShapedText, TextAlign, TextStyle};
use parking_lot::Mutex;
use std::hash::{Hash, Hasher};

const UNBOUNDED: f64 = 1_000_000_000.0;
/// Width of a textarea's vertical scrollbar, plus the gap between it and the text.
pub const SCROLLBAR_W: f64 = 10.0;
const SCROLLBAR_GAP: f64 = 2.0;

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

/// The advance of one space, measured between letters: trailing spaces shape to no advance, so
/// they can't be measured directly.
pub fn space_advance(fs: &mut dyn FontSystem, font_info: &FontInfo) -> f64 {
    (width(fs, "a a", font_info) - width(fs, "aa", font_info)).max(0.0)
}

/// Advance width of `text` including its trailing spaces. Shapers disagree on those (Pango
/// drops their advance, Parley keeps it), so measure without them and add real space advances.
fn width_with_trailing(fs: &mut dyn FontSystem, text: &str, font_info: &FontInfo) -> f64 {
    let trimmed = text.trim_end_matches(' ');
    let trailing = text.chars().count() - trimmed.chars().count();
    let mut w = width(fs, trimmed, font_info);
    if trailing > 0 {
        w += trailing as f64 * space_advance(fs, font_info);
    }
    w
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
        width_with_trailing(fs, last_line, font_info).min(avail)
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
///
/// The painter and the engine both ask for the rows of the same text several times per
/// keystroke, so the last few results are cached.
pub fn layout_rows(fs: &mut dyn FontSystem, text: &str, font_info: &FontInfo, w: f64) -> Vec<Row> {
    static CACHE: Mutex<Vec<(u64, usize, Vec<Row>)>> = Mutex::new(Vec::new());
    const CACHE_SIZE: usize = 8;

    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    w.to_bits().hash(&mut h);
    font_info.family.hash(&mut h);
    font_info.size.to_bits().hash(&mut h);
    font_info.weight.hash(&mut h);
    font_info.slant.hash(&mut h);
    font_info.letter_spacing.to_bits().hash(&mut h);
    let key = h.finish();
    {
        let mut cache = CACHE.lock();
        if let Some(i) = cache.iter().position(|(k, len, _)| *k == key && *len == text.len()) {
            let hit = cache.remove(i);
            let rows = hit.2.clone();
            cache.push(hit);
            return rows;
        }
    }
    let rows = layout_rows_uncached(fs, text, font_info, w);
    let mut cache = CACHE.lock();
    if cache.len() >= CACHE_SIZE {
        cache.remove(0);
    }
    cache.push((key, text.len(), rows.clone()));
    rows
}

fn layout_rows_uncached(fs: &mut dyn FontSystem, text: &str, font_info: &FontInfo, w: f64) -> Vec<Row> {
    let mut rows = Vec::new();
    let space_w = space_advance(fs, font_info);
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

/// The visual layout of a `<textarea>`'s content box: its rows, how many fit, where the text
/// block sits, and the scrollbar track when the rows overflow. Both the painter and the engine
/// build this from the same inputs, so what is drawn is what clicks and wheels map against.
#[derive(Debug, Clone)]
pub struct AreaLayout {
    pub rows: Vec<Row>,
    pub rows_fit: usize,
    pub line_h: f64,
    /// The text block (content box minus insets and the scrollbar).
    pub text: Rect,
    /// Scrollbar track at the right edge of the content box.
    pub track: Option<Rect>,
}

pub fn area_layout(fs: &mut dyn FontSystem, text: &str, font_info: &FontInfo, content: Rect) -> AreaLayout {
    let line_h = font_info.line_height.max(font_info.size);
    let rows_fit = (((content.height + 0.5) / line_h).floor() as usize).max(1);
    let inset = inset_x(content.width);
    let full = Rect::new(
        content.x + inset,
        content.y,
        (content.width - inset * 2.0).max(1.0),
        content.height.max(line_h),
    );
    let rows = layout_rows(fs, text, font_info, full.width);
    if rows.len() <= rows_fit {
        return AreaLayout {
            rows,
            rows_fit,
            line_h,
            text: full,
            track: None,
        };
    }
    // The rows overflow: give the scrollbar its strip and re-wrap in what is left.
    let bar = Rect::new(
        content.x + content.width - SCROLLBAR_W,
        content.y,
        SCROLLBAR_W,
        content.height,
    );
    let narrow = Rect::new(
        full.x,
        full.y,
        (full.width - SCROLLBAR_W - SCROLLBAR_GAP).max(1.0),
        full.height,
    );
    AreaLayout {
        rows: layout_rows(fs, text, font_info, narrow.width),
        rows_fit,
        line_h,
        text: narrow,
        track: Some(bar),
    }
}

impl AreaLayout {
    pub fn max_first(&self) -> usize {
        self.rows.len().saturating_sub(self.rows_fit)
    }

    /// `first` clamped to what can be scrolled to.
    pub fn clamp_first(&self, first: usize) -> usize {
        first.min(self.max_first())
    }

    /// The smallest change to `first` that brings `caret_row` into view.
    pub fn first_showing(&self, first: usize, caret_row: usize) -> usize {
        let first = self.clamp_first(first);
        if caret_row < first {
            caret_row
        } else if caret_row >= first + self.rows_fit {
            caret_row + 1 - self.rows_fit
        } else {
            first
        }
    }

    /// Index of the row at vertical offset `y` from the top of the text block, given `first`.
    pub fn row_at(&self, first: usize, y: f64) -> usize {
        let i = first + (y / self.line_h).floor().max(0.0) as usize;
        i.min(self.rows.len().saturating_sub(1))
    }

    /// Scrollbar thumb for `first`.
    pub fn thumb(&self, first: usize) -> Option<Rect> {
        let track = self.track?;
        let thumb_h = (track.height * self.rows_fit as f64 / self.rows.len() as f64).max(12.0);
        let travel = (track.height - thumb_h).max(0.0);
        let y = track.y + travel * (self.clamp_first(first) as f64 / self.max_first().max(1) as f64);
        Some(Rect::new(track.x + 2.0, y, track.width - 4.0, thumb_h))
    }

    /// The `first` row for a thumb dragged by `dy` px from where it was at `first`.
    pub fn first_for_thumb_drag(&self, first: usize, dy: f64) -> usize {
        let (Some(track), Some(thumb)) = (self.track, self.thumb(first)) else {
            return first;
        };
        let travel = (track.height - thumb.height).max(1.0);
        let frac = ((thumb.y + dy - track.y) / travel).clamp(0.0, 1.0);
        (frac * self.max_first() as f64).round() as usize
    }
}

/// First visible row so that `rows_fit` rows show `caret_row`.
pub fn first_visible_row(total: usize, caret_row: usize, rows_fit: usize) -> usize {
    let rows_fit = rows_fit.max(1);
    if total <= rows_fit {
        return 0;
    }
    caret_row.saturating_sub(rows_fit - 1).min(total - rows_fit)
}

/// Horizontal `[x0, x1)` of the selected part of a row (chars `[sel_start, sel_end)` of the whole
/// text), `None` when the selection doesn't touch the row. A selection that runs past the row's
/// end shows a little extra, like the newline/space it covers.
pub fn selection_in_row(
    fs: &mut dyn FontSystem,
    text: &str,
    row: &Row,
    sel: (usize, usize),
    font_info: &FontInfo,
) -> Option<(f64, f64)> {
    let (s, e) = sel;
    if e < row.start || s > row.end || s == e {
        return None;
    }
    let rt = row_text(text, row);
    let x0 = x_in_row(fs, &rt, s.saturating_sub(row.start), font_info);
    let mut x1 = x_in_row(fs, &rt, e.min(row.end) - row.start, font_info);
    if e > row.end {
        x1 += space_advance(fs, font_info);
    }
    Some((x0, x1.max(x0 + 1.0)))
}

/// Horizontal caret offset within a row's text. Trailing spaces of the prefix count with their
/// real advance so a caret sitting after a space (word jumps, mid-space clicks) lines up.
pub fn x_in_row(fs: &mut dyn FontSystem, row_text: &str, chars: usize, font_info: &FontInfo) -> f64 {
    let prefix: String = row_text.chars().take(chars).collect();
    if prefix.is_empty() {
        return 0.0;
    }
    width_with_trailing(fs, &prefix, font_info)
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
    // Trailing spaces must count, or every boundary inside a space run measures the same and
    // clicks there all land on the run's first space.
    let mut prefix_w = |i: usize| -> f64 {
        let p: String = text.chars().take(i).collect();
        width_with_trailing(fs, &p, font_info)
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
