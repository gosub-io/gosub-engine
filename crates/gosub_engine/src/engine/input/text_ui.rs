//! Text-control UI on top of [`edit`]: the caret from clicks, drag / double-click selection,
//! row navigation and scrolling in textareas, and the clipboard handshake with the embedder.
//! Geometry comes from [`text_field`], the same code the painter draws with.

use super::select_ui::popup_lei;
use super::{hit_at, layout_element_of, resize_grip_hit, InputHost, PageInput};
use crate::engine::edit::{self, EditAction, Motion};
use crate::engine::events::CursorShape;
use gosub_interface::document::{ControlEditState, Document as _};
use gosub_render_pipeline::common::font::FontInfo;
use gosub_render_pipeline::common::geo::Rect;
use gosub_render_pipeline::layering::layer::LayerList;
use gosub_render_pipeline::layouter::{ElementContext, FormControl, LayoutElementId};
use gosub_render_pipeline::painter::text_field;
use gosub_shared::node::NodeId;
use std::time::{Duration, Instant};

/// Presses closer together than this (in time and distance) count as a multi-click.
const MULTI_CLICK: Duration = Duration::from_millis(400);
const MULTI_CLICK_SLOP: f64 = 4.0;

/// What the layouter knows about a text control's box.
struct TextGeometry {
    font_info: FontInfo,
    masked: bool,
    multiline: bool,
    content: Rect,
}

fn text_geometry(layer_list: Option<&LayerList>, lei: LayoutElementId) -> Option<TextGeometry> {
    let el = layer_list?.layout_tree.get_node_by_id(lei)?;
    let ElementContext::FormControl(fc) = &el.context else {
        return None;
    };
    let FormControl::TextField { masked, multiline, .. } = &fc.control else {
        return None;
    };
    Some(TextGeometry {
        font_info: fc.font_info.clone(),
        masked: *masked,
        multiline: *multiline,
        content: el.box_model.content_box,
    })
}

/// What the painter draws for `state`: bullets for a password field.
fn shown_text(state: &ControlEditState, masked: bool) -> String {
    if masked {
        "\u{2022}".repeat(state.value.chars().count())
    } else {
        state.value.clone()
    }
}

impl PageInput {
    /// The control's edit state, or a fresh one from the markup: a textarea starts with the caret
    /// at the top (its scroll is 0), a single-line field at the end.
    pub(super) fn edit_state<H: InputHost>(host: &H, node: NodeId) -> ControlEditState {
        let Some(doc) = host.document() else {
            return ControlEditState::new(String::new(), 0);
        };
        doc.control_edit_state(node).unwrap_or_else(|| {
            let value = edit::initial_value(&doc, node);
            let caret = if doc.tag_name(node) == Some("textarea") {
                0
            } else {
                value.chars().count()
            };
            ControlEditState::new(value, caret)
        })
    }

    /// Store `state` (keeping a textarea's caret inside its view) and repaint the control.
    pub(super) fn commit_edit_state<H: InputHost>(&mut self, host: &mut H, node: NodeId, mut state: ControlEditState) {
        let Some(doc) = host.document() else {
            return;
        };
        let ll = host.layer_list();
        if let Some(geo) = layout_element_of(ll.as_deref(), node).and_then(|lei| text_geometry(ll.as_deref(), lei)) {
            if geo.multiline {
                let fs = host.font_system();
                let mut fs = fs.lock();
                let shown = shown_text(&state, geo.masked);
                let area = text_field::area_layout(&mut *fs, &shown, &geo.font_info, geo.content);
                let caret_row = text_field::row_of_caret(&area.rows, state.caret);
                state.scroll = area.first_showing(state.scroll, caret_row);
            }
        }
        doc.set_control_edit_state(node, Some(state));
        Self::request_repaint(host, node);
    }

    /// Char index under a viewport point in text control `node`, mirroring the painter's window
    /// (insets, single-line horizontal scroll, textarea rows + scroll). `None` on the scrollbar.
    fn caret_index_at<H: InputHost>(
        host: &H,
        node: NodeId,
        lei: LayoutElementId,
        vp_x: f64,
        vp_y: f64,
    ) -> Option<usize> {
        let geo = text_geometry(host.layer_list().as_deref(), lei)?;
        let state = Self::edit_state(host, node);
        if state.value.is_empty() {
            return Some(0);
        }
        let shown = shown_text(&state, geo.masked);
        let (scroll_x, scroll_y) = host.scroll();
        let (px, py) = (vp_x + scroll_x, vp_y + scroll_y);
        let fs = host.font_system();
        let mut fs = fs.lock();
        if geo.multiline {
            let area = text_field::area_layout(&mut *fs, &shown, &geo.font_info, geo.content);
            if area.track.is_some_and(|t| t.contains(px, py)) {
                return None;
            }
            let first = area.clamp_first(state.scroll);
            // Dragging past the edge reaches one more row than is shown, which scrolls by one.
            let y = (py - area.text.y).clamp(-area.line_h, area.rows_fit as f64 * area.line_h);
            let row = &area.rows[area.row_at(first, y)];
            let rt = text_field::row_text(&shown, row);
            let x = px - area.text.x;
            let in_row = text_field::index_at_x(&mut *fs, &rt, &geo.font_info, x).min(row.end - row.start);
            return Some(row.start + in_row);
        }
        let inset = text_field::inset_x(geo.content.width);
        let width = (geo.content.width - inset * 2.0).max(1.0);
        let x = px - geo.content.x - inset;
        let (start, end) = text_field::single_line_window(&mut *fs, &shown, Some(state.caret), &geo.font_info, width);
        let visible: String = shown.chars().skip(start).take(end - start).collect();
        Some(start + text_field::index_at_x(&mut *fs, &visible, &geo.font_info, x))
    }

    /// A press straight into text control `node`: put the caret there (collapsing any selection).
    /// Paint-only.
    pub(super) fn place_caret<H: InputHost>(
        &mut self,
        host: &mut H,
        node: NodeId,
        lei: LayoutElementId,
        vp_x: f64,
        vp_y: f64,
    ) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        if edit::text_entry_kind(&doc, node).is_none() {
            return false;
        }
        let Some(idx) = Self::caret_index_at(host, node, lei, vp_x, vp_y) else {
            return false;
        };
        let mut state = Self::edit_state(host, node);
        let changed = edit::apply(
            &mut state,
            &EditAction::Move {
                motion: Motion::To(idx),
                extend: false,
            },
        );
        if !changed && doc.control_edit_state(node).is_some() {
            return false;
        }
        self.commit_edit_state(host, node, state);
        true
    }

    /// The rest of a press on a text control (after the caret was placed): a textarea scrollbar
    /// press scrolls/drags, a double click selects the word, a triple click everything, and a
    /// single press starts a drag selection.
    pub(super) fn text_press<H: InputHost>(
        &mut self,
        host: &mut H,
        node: NodeId,
        lei: LayoutElementId,
        vp_x: f64,
        vp_y: f64,
    ) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        if edit::text_entry_kind(&doc, node).is_none() {
            return false;
        }
        if self.area_track_press(host, node, lei, vp_x, vp_y) {
            return true;
        }
        let now = Instant::now();
        let count = match self.last_press {
            Some((at, x, y, n))
                if now.duration_since(at) < MULTI_CLICK
                    && (x - vp_x).abs() <= MULTI_CLICK_SLOP
                    && (y - vp_y).abs() <= MULTI_CLICK_SLOP =>
            {
                n + 1
            }
            _ => 1,
        };
        self.last_press = Some((now, vp_x, vp_y, count));
        let mut state = Self::edit_state(host, node);
        match count {
            1 => {
                self.drag_select = Some((node, lei));
                false
            }
            2 => {
                let (s, e) = edit::word_at(&state.value, state.caret);
                state.anchor = (s != e).then_some(s);
                state.caret = e;
                self.commit_edit_state(host, node, state);
                true
            }
            _ => {
                edit::apply(&mut state, &EditAction::SelectAll);
                self.commit_edit_state(host, node, state);
                true
            }
        }
    }

    /// A press on a textarea's scrollbar: on the thumb starts a drag, on the track pages.
    fn area_track_press<H: InputHost>(
        &mut self,
        host: &mut H,
        node: NodeId,
        lei: LayoutElementId,
        vp_x: f64,
        vp_y: f64,
    ) -> bool {
        let Some(geo) = text_geometry(host.layer_list().as_deref(), lei) else {
            return false;
        };
        if !geo.multiline {
            return false;
        }
        let state = Self::edit_state(host, node);
        let (scroll_x, scroll_y) = host.scroll();
        let (px, py) = (vp_x + scroll_x, vp_y + scroll_y);
        let area = {
            let fs = host.font_system();
            let mut fs = fs.lock();
            text_field::area_layout(&mut *fs, &shown_text(&state, geo.masked), &geo.font_info, geo.content)
        };
        let Some(track) = area.track.filter(|t| t.contains(px, py)) else {
            return false;
        };
        let first = area.clamp_first(state.scroll);
        let Some(thumb) = area.thumb(first) else {
            return false;
        };
        if thumb.contains(px, py) {
            self.drag_area_thumb = Some((node, lei, vp_y, first));
            return true;
        }
        let _ = track;
        let target = if py < thumb.y {
            first.saturating_sub(area.rows_fit)
        } else {
            first + area.rows_fit
        };
        Self::set_area_scroll(host, node, area.clamp_first(target))
    }

    fn set_area_scroll<H: InputHost>(host: &mut H, node: NodeId, first: usize) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let mut state = Self::edit_state(host, node);
        if state.scroll == first && doc.control_edit_state(node).is_some() {
            return false;
        }
        state.scroll = first;
        doc.set_control_edit_state(node, Some(state));
        Self::request_repaint(host, node);
        true
    }

    /// Pointer moved with the button held after a press in a text control: extend the selection
    /// to the char under the pointer. Paint-only.
    pub(super) fn drag_select_to<H: InputHost>(&mut self, host: &mut H, vp_x: f64, vp_y: f64) -> bool {
        let Some((node, lei)) = self.drag_select else {
            return false;
        };
        let Some(idx) = Self::caret_index_at(host, node, lei, vp_x, vp_y) else {
            return false;
        };
        let mut state = Self::edit_state(host, node);
        let changed = edit::apply(
            &mut state,
            &EditAction::Move {
                motion: Motion::To(idx),
                extend: true,
            },
        );
        if !changed {
            return false;
        }
        self.commit_edit_state(host, node, state);
        true
    }

    /// Textarea scrollbar thumb being dragged: `start_y`/`start_first` from the press.
    pub(super) fn area_thumb_drag_to<H: InputHost>(&mut self, host: &mut H, vp_y: f64) -> bool {
        let Some((node, lei, start_y, start_first)) = self.drag_area_thumb else {
            return false;
        };
        let Some(geo) = text_geometry(host.layer_list().as_deref(), lei) else {
            return false;
        };
        let state = Self::edit_state(host, node);
        let area = {
            let fs = host.font_system();
            let mut fs = fs.lock();
            text_field::area_layout(&mut *fs, &shown_text(&state, geo.masked), &geo.font_info, geo.content)
        };
        let first = area.first_for_thumb_drag(start_first, vp_y - start_y);
        Self::set_area_scroll(host, node, first)
    }

    /// Wheel over a textarea whose rows overflow scrolls it by `delta_y` px (~3 rows per notch
    /// of 120). Returns whether the wheel was consumed.
    pub fn area_scroll<H: InputHost>(host: &mut H, vp_x: f64, vp_y: f64, delta_y: f64) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let ll = host.layer_list();
        let (Some(leaf), Some(lei)) = hit_at(ll.as_deref(), host.scroll(), vp_x, vp_y) else {
            return false;
        };
        if edit::text_entry_kind(&doc, leaf) != Some(true) {
            return false;
        }
        let Some(geo) = text_geometry(ll.as_deref(), lei) else {
            return false;
        };
        let state = Self::edit_state(host, leaf);
        let area = {
            let fs = host.font_system();
            let mut fs = fs.lock();
            text_field::area_layout(&mut *fs, &shown_text(&state, geo.masked), &geo.font_info, geo.content)
        };
        if area.track.is_none() {
            return false;
        }
        let rows = (delta_y / 40.0).round() as i64;
        let rows = if rows == 0 { delta_y.signum() as i64 } else { rows };
        let first = (area.clamp_first(state.scroll) as i64 + rows).clamp(0, area.max_first() as i64) as usize;
        // Consume the wheel even at the ends so the page doesn't scroll under the textarea.
        Self::set_area_scroll(host, leaf, first);
        true
    }

    /// Row-based keys in a textarea (`ArrowUp`/`ArrowDown`/`PageUp`/`PageDown`/`Home`/`End`);
    /// `None` when `key` isn't one. Up/Down keep the caret's x on the new row.
    pub(super) fn row_key<H: InputHost>(&mut self, host: &mut H, node: NodeId, key: &str, shift: bool) -> Option<bool> {
        if !matches!(key, "ArrowUp" | "ArrowDown" | "PageUp" | "PageDown" | "Home" | "End") {
            return None;
        }
        let ll = host.layer_list();
        let lei = layout_element_of(ll.as_deref(), node)?;
        let geo = text_geometry(ll.as_deref(), lei)?;
        let mut state = Self::edit_state(host, node);
        let shown = shown_text(&state, geo.masked);
        let target = {
            let fs = host.font_system();
            let mut fs = fs.lock();
            let area = text_field::area_layout(&mut *fs, &shown, &geo.font_info, geo.content);
            let row_i = text_field::row_of_caret(&area.rows, state.caret);
            let row = &area.rows[row_i];
            let last = area.rows.len() - 1;
            let mut to_row = |i: Option<usize>| -> usize {
                let Some(i) = i else {
                    return 0;
                };
                let Some(r) = area.rows.get(i) else {
                    return state.value.chars().count();
                };
                let x = text_field::x_in_row(
                    &mut *fs,
                    &text_field::row_text(&shown, row),
                    state.caret - row.start,
                    &geo.font_info,
                );
                let rt = text_field::row_text(&shown, r);
                r.start + text_field::index_at_x(&mut *fs, &rt, &geo.font_info, x).min(r.end - r.start)
            };
            match key {
                "Home" => row.start,
                "End" => row.end,
                "ArrowUp" => to_row(row_i.checked_sub(1)),
                "ArrowDown" => to_row((row_i < last).then_some(row_i + 1)),
                "PageUp" => to_row(Some(row_i.saturating_sub(area.rows_fit))),
                _ => to_row(Some((row_i + area.rows_fit).min(last))),
            }
        };
        let changed = edit::apply(
            &mut state,
            &EditAction::Move {
                motion: Motion::To(target),
                extend: shift,
            },
        );
        if changed {
            self.commit_edit_state(host, node, state);
        }
        Some(changed)
    }

    /// Ctrl/Cmd+C/X/V on a text control. Copy/cut hand the text to the embedder through
    /// [`take_clipboard_write`](Self::take_clipboard_write); paste asks for the clipboard
    /// ([`take_paste_request`](Self::take_paste_request)), which comes back as `TextInput`.
    /// `None` when `key` isn't a clipboard chord.
    pub(super) fn clipboard_key<H: InputHost>(
        &mut self,
        host: &mut H,
        node: NodeId,
        key: &str,
        masked: bool,
    ) -> Option<bool> {
        match key {
            "c" | "C" | "x" | "X" => {
                let mut state = Self::edit_state(host, node);
                let Some((s, e)) = state.selection() else {
                    return Some(false);
                };
                // A password field never gives its text away.
                if !masked {
                    self.clipboard_write = Some(state.value.chars().skip(s).take(e - s).collect());
                }
                if key.eq_ignore_ascii_case("x") && edit::apply(&mut state, &EditAction::Insert(String::new())) {
                    self.commit_edit_state(host, node, state);
                    return Some(true);
                }
                Some(false)
            }
            "v" | "V" => {
                self.paste_requested = true;
                Some(false)
            }
            _ => None,
        }
    }

    /// Text the page wants on the clipboard (Ctrl+C / Ctrl+X), once.
    pub fn take_clipboard_write(&mut self) -> Option<String> {
        self.clipboard_write.take()
    }

    /// Whether the page asked for a paste (Ctrl+V) since the last call; answer with the
    /// clipboard text as `TextInput`.
    pub fn take_paste_request(&mut self) -> bool {
        std::mem::take(&mut self.paste_requested)
    }

    /// What the mouse cursor should be at a viewport point: I-beam over editable text (but not
    /// its scrollbar), resize arrows over a textarea's grip, a pointing hand over links, default
    /// elsewhere. An active drag keeps its cursor even when the pointer strays off the control.
    pub fn cursor_at<H: InputHost>(&self, host: &H, vp_x: f64, vp_y: f64) -> CursorShape {
        if self.drag_resize.is_some() {
            return CursorShape::Resize;
        }
        if self.drag_select.is_some() {
            return CursorShape::Text;
        }
        let Some(doc) = host.document() else {
            // A remotely rendered page: no document here, but the hover just
            // updated from the renderer's hit regions says what is under the
            // pointer (a link, text), and that is the cursor.
            return host.hover_cursor();
        };
        let ll = host.layer_list();
        let (leaf, lei) = hit_at(ll.as_deref(), host.scroll(), vp_x, vp_y);
        // Popup rows are picked with a plain arrow.
        if doc.open_select().is_some() && popup_lei(ll.as_deref()) == lei {
            return CursorShape::Default;
        }
        if host.hover_has_link() {
            return CursorShape::Pointer;
        }
        let (Some(leaf), Some(lei)) = (leaf, lei) else {
            return CursorShape::Default;
        };
        if edit::text_entry_kind(&doc, leaf).is_none() {
            return CursorShape::Default;
        }
        if resize_grip_hit(ll.as_deref(), host.scroll(), leaf, lei, vp_x, vp_y).is_some() {
            return CursorShape::Resize;
        }
        // The scrollbar strip of an overflowing textarea is not text.
        if Self::caret_index_at(host, leaf, lei, vp_x, vp_y).is_none() {
            return CursorShape::Default;
        }
        CursorShape::Text
    }

    /// Keyboard focus landing on a single-line text field selects its text (Tab behaviour).
    pub(super) fn select_all_on_focus<H: InputHost>(host: &H, node: NodeId) {
        let Some(doc) = host.document() else {
            return;
        };
        if edit::text_entry_kind(&doc, node) != Some(false) {
            return;
        }
        let mut state = Self::edit_state(host, node);
        edit::apply(&mut state, &EditAction::SelectAll);
        doc.set_control_edit_state(node, Some(state));
    }
}
