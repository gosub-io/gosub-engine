//! `<select>` dropdown interaction: opening/closing, row picking, hover/keyboard highlight,
//! scrolling (wheel, scrollbar drag, paging), keyboard navigation and type-ahead. The popup's
//! geometry comes from the layout tree (`LayoutTree::popup`); its state lives on the document
//! (`OpenSelect`) where the painter reads it.

use super::BrowsingContext;
use crate::engine::edit;
use crate::html::RenderConfiguration;
use cow_utils::CowUtils;
use gosub_interface::document::{Document as _, OpenSelect};
use gosub_render_pipeline::common::geo::Rect;
use gosub_render_pipeline::layouter::{popup_placement, ElementContext, ElementContextSelectPopup, LayoutElementId};
use gosub_shared::node::NodeId;
use std::time::{Duration, Instant};

/// Keys typed within this window extend the type-ahead prefix; later ones start a new one.
const TYPEAHEAD_WINDOW: Duration = Duration::from_millis(1000);

impl<C: RenderConfiguration> BrowsingContext<C> {
    /// The open popup's layout element, content box and context.
    fn popup_parts(&self) -> Option<(LayoutElementId, Rect, &ElementContextSelectPopup)> {
        let ll = self.active_layer_list()?;
        let popup = ll.layout_tree.popup?;
        let el = ll.layout_tree.get_node_by_id(popup)?;
        match &el.context {
            ElementContext::SelectPopup(ctx) => Some((popup, el.box_model.content_box, ctx)),
            _ => None,
        }
    }

    fn popup_lei(&self) -> Option<LayoutElementId> {
        self.active_layer_list().and_then(|ll| ll.layout_tree.popup)
    }

    fn set_open(&mut self, open: OpenSelect) {
        if let Some(doc) = &self.document {
            doc.set_open_select(Some(open));
        }
    }

    /// Paint-only repaint of the popup after a highlight/scroll change.
    fn repaint_popup(&mut self) {
        if let Some(popup) = self.popup_lei() {
            self.paint_dirty_leis.push(popup);
        }
    }

    /// Open `select`'s dropdown with its current choice in view and keyboard-active.
    pub(super) fn open_select_popup(&mut self, select: NodeId) -> bool {
        let Some(doc) = self.document.clone() else {
            return false;
        };
        let viewport = (self.scroll_y, self.viewport.height as f64);
        // Decide the row window the way the layouter will, so the choice lands mid-popup.
        let rows = popup_rows_of(&doc, select);
        let chosen = doc.selected_option(select);
        let chosen_row = rows.iter().find(|(n, _)| Some(*n) == chosen).map(|(_, row)| *row);
        let total_rows = rows.last().map_or(0, |(_, r)| r + 1);
        let anchor = self.select_anchor(select).unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
        let (_, visible) = popup_placement(anchor, viewport, total_rows);
        let first_row = chosen_row.map_or(0, |r| {
            r.saturating_sub(visible / 2).min(total_rows.saturating_sub(visible))
        });
        self.set_open(OpenSelect {
            select,
            hover: None,
            active: chosen_row,
            first_row,
            viewport,
        });
        self.invalidate_render();
        true
    }

    pub(super) fn close_select_popup(&mut self) {
        if let Some(doc) = &self.document {
            doc.set_open_select(None);
        }
        self.invalidate_render();
    }

    /// Border box of the select in page px.
    fn select_anchor(&self, select: NodeId) -> Option<Rect> {
        let ll = self.active_layer_list()?;
        ll.layout_tree
            .arena
            .values()
            .find(|el| el.dom_node_id == select && matches!(el.context, ElementContext::FormControl(_)))
            .map(|el| el.box_model.border_box)
    }

    /// A press while a dropdown is open: scrollbar (thumb drag / page), a selectable row
    /// (commit), or anything else (close without changing).
    pub(super) fn popup_press(&mut self, lei: Option<LayoutElementId>, vp_x: f64, vp_y: f64) -> bool {
        if self.popup_scrollbar_press(lei, vp_x, vp_y) {
            return true;
        }
        if let Some(row) = self.popup_row_at(lei, vp_x, vp_y) {
            self.commit_row(row);
            return true;
        }
        self.close_select_popup();
        true
    }

    /// Commit popup row `row` (if it is a selectable option) and close.
    fn commit_row(&mut self, row: usize) {
        let Some(doc) = self.document.clone() else {
            return;
        };
        let Some(open) = doc.open_select() else {
            return;
        };
        if let Some((_, _, ctx)) = self.popup_parts() {
            if let Some(r) = ctx.rows.get(row) {
                if r.selectable() {
                    if let Some(id) = r.option_id() {
                        doc.set_selected_option(open.select, Some(id));
                    }
                }
            }
        }
        self.close_select_popup();
    }

    /// The selectable row under a point on the popup (not its scrollbar), if any.
    fn popup_row_at(&self, lei: Option<LayoutElementId>, vp_x: f64, vp_y: f64) -> Option<usize> {
        let (popup, inner, ctx) = self.popup_parts()?;
        if lei != Some(popup) {
            return None;
        }
        let (x, y) = (vp_x + self.scroll_x, vp_y + self.scroll_y);
        if x >= inner.x + inner.width - ctx.scrollbar_width() {
            return None;
        }
        let first = self
            .document
            .as_ref()
            .and_then(|d| d.open_select())
            .map_or(0, |o| o.first_row);
        let row = ((y - inner.y) / ctx.row_height).floor();
        if row < 0.0 || row as usize >= ctx.visible_rows {
            return None;
        }
        let row = first + row as usize;
        ctx.rows.get(row).filter(|r| r.selectable()).map(|_| row)
    }

    /// Pointer over an open dropdown: light-highlight the row under it (paint-only).
    pub fn popup_hover_at(&mut self, vp_x: f64, vp_y: f64) -> bool {
        let Some(doc) = self.document.clone() else {
            return false;
        };
        let Some(open) = doc.open_select() else {
            return false;
        };
        let (_, lei) = self.hit_at(vp_x, vp_y);
        let row = self.popup_row_at(lei, vp_x, vp_y);
        if row == open.hover {
            return false;
        }
        self.set_open(OpenSelect { hover: row, ..open });
        self.repaint_popup();
        true
    }

    /// Mouse wheel over an open dropdown: scroll its list one row per notch (paint-only).
    pub fn popup_scroll(&mut self, vp_x: f64, vp_y: f64, delta_y: f64) -> bool {
        let Some(doc) = self.document.clone() else {
            return false;
        };
        let Some(open) = doc.open_select() else {
            return false;
        };
        let Some((popup, _, ctx)) = self.popup_parts() else {
            return false;
        };
        if self.hit_at(vp_x, vp_y).1 != Some(popup) {
            return false;
        }
        let max_first = ctx.max_first_row();
        let step = if delta_y > 0.0 { 1 } else { -1 };
        let first = (open.first_row as isize + step).clamp(0, max_first as isize) as usize;
        self.set_open(OpenSelect {
            first_row: first,
            ..open
        });
        // The row under the pointer changes as the list moves.
        let hover = self.popup_row_at(Some(popup), vp_x, vp_y);
        self.set_open(OpenSelect {
            first_row: first,
            hover,
            ..open
        });
        self.repaint_popup();
        true
    }

    /// A press on the popup's scrollbar: on the thumb starts a drag, on the track pages.
    fn popup_scrollbar_press(&mut self, lei: Option<LayoutElementId>, vp_x: f64, vp_y: f64) -> bool {
        let Some(doc) = self.document.clone() else {
            return false;
        };
        let Some(open) = doc.open_select() else {
            return false;
        };
        let Some((popup, inner, ctx)) = self.popup_parts() else {
            return false;
        };
        if lei != Some(popup) {
            return false;
        }
        let Some((track, thumb)) = ctx.scrollbar(inner, open.first_row) else {
            return false;
        };
        let (x, y) = (vp_x + self.scroll_x, vp_y + self.scroll_y);
        if x < track.x {
            return false;
        }
        if y >= thumb.y && y < thumb.y + thumb.height {
            self.drag_popup_thumb = Some((vp_y, open.first_row));
            return true;
        }
        let (visible, max_first) = (ctx.visible_rows, ctx.max_first_row());
        let first = if y < thumb.y {
            open.first_row.saturating_sub(visible)
        } else {
            (open.first_row + visible).min(max_first)
        };
        self.set_open(OpenSelect {
            first_row: first,
            ..open
        });
        self.repaint_popup();
        true
    }

    /// Thumb drag: pointer travel over the track's free length maps onto the row range.
    pub(super) fn popup_thumb_drag_to(&mut self, start_y: f64, start_first: usize, vp_y: f64) -> bool {
        let Some(doc) = self.document.clone() else {
            return false;
        };
        let Some(open) = doc.open_select() else {
            return false;
        };
        let Some((_, inner, ctx)) = self.popup_parts() else {
            return false;
        };
        let Some((track, thumb)) = ctx.scrollbar(inner, open.first_row) else {
            return false;
        };
        let travel = (track.height - thumb.height).max(1.0);
        let max_first = ctx.max_first_row() as f64;
        let first = (start_first as f64 + (vp_y - start_y) / travel * max_first)
            .round()
            .clamp(0.0, max_first) as usize;
        if first == open.first_row {
            return false;
        }
        self.set_open(OpenSelect {
            first_row: first,
            ..open
        });
        self.repaint_popup();
        true
    }

    /// Keyboard on a focused `<select>`, per the design guide: closed, arrows/Home/End change
    /// the selection and Enter/Space/Alt+↓ open; open, arrows/PgUp/PgDn/Home/End move the
    /// active row, Enter/Space commit it, Esc/Alt+↑ close without changing. Letters type ahead.
    pub(super) fn select_key(&mut self, select: NodeId, key: &str, alt: bool) -> bool {
        let Some(doc) = self.document.clone() else {
            return false;
        };
        let open = doc.open_select();

        if key.chars().count() == 1 && !key.chars().next().is_some_and(char::is_control) {
            return self.select_typeahead(select, key);
        }
        self.typeahead = None;

        match (open, key) {
            (Some(_), "Escape") => {
                self.close_select_popup();
                true
            }
            (Some(_), "ArrowUp") if alt => {
                self.close_select_popup();
                true
            }
            (Some(o), "Enter" | " ") => {
                match o.active {
                    Some(row) => self.commit_row(row),
                    None => self.close_select_popup(),
                }
                true
            }
            (Some(o), "ArrowDown" | "ArrowUp" | "PageDown" | "PageUp" | "Home" | "End") => {
                let Some((_, _, ctx)) = self.popup_parts() else {
                    return false;
                };
                let selectable: Vec<usize> = ctx
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.selectable())
                    .map(|(i, _)| i)
                    .collect();
                if selectable.is_empty() {
                    return false;
                }
                let visible = ctx.visible_rows;
                let cur = o.active.and_then(|a| selectable.iter().position(|&i| i == a));
                let last = selectable.len() - 1;
                let next = match (key, cur) {
                    ("Home", _) => 0,
                    ("End", _) => last,
                    ("ArrowDown", Some(i)) => (i + 1).min(last),
                    ("ArrowUp", Some(i)) => i.saturating_sub(1),
                    ("PageDown", Some(i)) => (i + visible).min(last),
                    ("PageUp", Some(i)) => i.saturating_sub(visible),
                    (_, None) => 0,
                    _ => return false,
                };
                let row = selectable[next];
                self.set_open(OpenSelect {
                    active: Some(row),
                    first_row: scrolled_into_view(o.first_row, row, visible, ctx.rows.len()),
                    ..o
                });
                self.repaint_popup();
                true
            }
            (None, "Enter" | " ") => self.open_select_popup(select),
            (None, "ArrowDown") if alt => self.open_select_popup(select),
            (None, "ArrowDown" | "ArrowUp" | "Home" | "End" | "PageDown" | "PageUp") => {
                let options = edit::select_options(&doc, select);
                if options.is_empty() {
                    return false;
                }
                let cur = doc
                    .selected_option(select)
                    .and_then(|c| options.iter().position(|&o| o == c));
                let last = options.len() - 1;
                let next = match (key, cur) {
                    ("Home", _) => 0,
                    ("End", _) => last,
                    ("ArrowDown", Some(i)) => (i + 1).min(last),
                    ("ArrowUp", Some(i)) => i.saturating_sub(1),
                    ("PageDown", Some(i)) => (i + 10).min(last),
                    ("PageUp", Some(i)) => i.saturating_sub(10),
                    (_, None) => 0,
                    _ => return false,
                };
                doc.set_selected_option(select, Some(options[next]));
                self.invalidate_render();
                true
            }
            _ => false,
        }
    }

    /// Type-ahead: letters typed in quick succession form a prefix; the first option starting
    /// with it becomes the selection (closed) or the active row (open). Repeating the same
    /// letter cycles through the options starting with it.
    fn select_typeahead(&mut self, select: NodeId, key: &str) -> bool {
        let Some(doc) = self.document.clone() else {
            return false;
        };
        let now = Instant::now();
        let mut prefix = match self.typeahead.take() {
            Some((p, at)) if now.duration_since(at) < TYPEAHEAD_WINDOW => p,
            _ => String::new(),
        };
        prefix.push_str(&key.cow_to_lowercase());
        self.typeahead = Some((prefix.clone(), now));

        let rows = popup_rows_of(&doc, select);
        if rows.is_empty() {
            return false;
        }
        let open = doc.open_select();
        let current = match open {
            Some(o) => o.active.and_then(|a| rows.iter().position(|(_, pos)| *pos == a)),
            None => doc
                .selected_option(select)
                .and_then(|c| rows.iter().position(|(n, _)| *n == c)),
        };
        // A repeated single letter cycles; anything longer matches the prefix from the top.
        let first_char = prefix.chars().next();
        let cycling = prefix.chars().count() > 1 && prefix.chars().all(|c| Some(c) == first_char);
        let needle: String = if cycling {
            prefix.chars().take(1).collect()
        } else {
            prefix.clone()
        };
        let start = if cycling { current.map_or(0, |i| i + 1) } else { 0 };
        let n = rows.len();
        let found = (0..n)
            .map(|k| (start + k) % n)
            .find(|&i| option_label(&doc, rows[i].0).cow_to_lowercase().starts_with(&needle));
        let Some(i) = found else {
            return true;
        };
        let (node, popup_row) = rows[i];
        match open {
            Some(o) => {
                let (visible, total) = self
                    .popup_parts()
                    .map_or((1, n), |(_, _, ctx)| (ctx.visible_rows, ctx.rows.len()));
                self.set_open(OpenSelect {
                    active: Some(popup_row),
                    first_row: scrolled_into_view(o.first_row, popup_row, visible, total),
                    ..o
                });
                self.repaint_popup();
            }
            None => {
                doc.set_selected_option(select, Some(node));
                self.invalidate_render();
            }
        }
        true
    }
}

/// Adjust `first` so `row` is inside a window of `visible` rows over `total`.
fn scrolled_into_view(first: usize, row: usize, visible: usize, total: usize) -> usize {
    let mut f = first;
    if row < f {
        f = row;
    } else if row >= f + visible {
        f = row + 1 - visible;
    }
    f.min(total.saturating_sub(visible))
}

/// The enabled options of `select` with their popup row index (group labels take a row each,
/// matching the layouter's `collect_rows`).
fn popup_rows_of<C: RenderConfiguration>(doc: &crate::html::EngineDocument<C>, select: NodeId) -> Vec<(NodeId, usize)> {
    let mut out = Vec::new();
    let mut row = 0usize;
    for &child in doc.children(select) {
        match doc.tag_name(child) {
            Some("optgroup") => {
                row += 1;
                for &opt in doc.children(child) {
                    if doc.tag_name(opt) == Some("option") {
                        if doc.attribute(opt, "disabled").is_none() {
                            out.push((opt, row));
                        }
                        row += 1;
                    }
                }
            }
            Some("option") => {
                if doc.attribute(child, "disabled").is_none() {
                    out.push((child, row));
                }
                row += 1;
            }
            _ => {}
        }
    }
    out
}

fn option_label<C: RenderConfiguration>(doc: &crate::html::EngineDocument<C>, option: NodeId) -> String {
    if let Some(l) = doc.attribute(option, "label").filter(|l| !l.is_empty()) {
        return l.to_string();
    }
    doc.children(option)
        .iter()
        .filter_map(|&c| doc.text_value(c))
        .collect::<String>()
        .trim()
        .to_string()
}
