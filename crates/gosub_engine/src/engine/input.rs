//! [`PageInput`]: the input layer of a page - focus, click activation, text editing,
//! `<select>` dropdowns, range sliders, pickers, drags and form submission - written against a
//! [host](InputHost) that owns the document, the layout and the damage. The tab's
//! [`BrowsingContext`](crate::engine::context::BrowsingContext) is one host; a renderer
//! process that retains a page can be another, so a page rendered out of process takes its
//! input where its DOM is.
//!
//! What stays with the host: hover (it is welded to the hit-test cache and, for a remote page,
//! to the renderer's hit regions), scrolling, and the context-menu hit test. What lives here
//! only reads the document and the layout through the host and reports what it changed as
//! damage, so neither side needs to know how the other renders.

use crate::engine::edit;
use crate::engine::events::{CursorShape, Modifiers, PickerKind};
use crate::engine::focus;
use crate::engine::form;
pub use crate::engine::form::Submission;
use crate::html::{is_text_input, EngineDocument, RenderConfiguration};
use gosub_interface::document::Document as _;
use gosub_interface::font_system::FontSystem;
use gosub_render_pipeline::common::document::pipeline_doc::pseudo_owner;
use gosub_render_pipeline::common::geo::Rect;
use gosub_render_pipeline::layering::layer::LayerList;
use gosub_render_pipeline::layouter::{ElementContext, FormControl, LayoutElementId, Resize};
use gosub_shared::node::NodeId;
use std::sync::Arc;

mod select_ui;
mod text_ui;

/// What the input layer needs from the process that holds the page.
pub(crate) trait InputHost {
    type Config: RenderConfiguration;

    /// The page's document, when this process holds one.
    fn document(&self) -> Option<Arc<EngineDocument<Self::Config>>>;
    /// The layer list the page was last laid out into, for hit testing and geometry.
    fn layer_list(&self) -> Option<Arc<LayerList>>;
    /// The scroll offset in CSS px: what viewport coordinates are measured against.
    fn scroll(&self) -> (f64, f64);
    /// The viewport height in CSS px, for placing a dropdown.
    fn viewport_height(&self) -> f64;
    /// The font system text measurements use, the one the painter draws with.
    fn font_system(&self) -> Arc<parking_lot::Mutex<dyn FontSystem>>;
    /// Whether the host's last hover found a link under the pointer.
    fn hover_has_link(&self) -> bool;
    /// The cursor the host's last hover resolved; what a page without a document here
    /// (rendered out of process) answers with.
    fn hover_cursor(&self) -> CursorShape;
    /// Paint damage over these elements' margin boxes: the repaint touches every tile they
    /// overlap and no others.
    fn repaint_elements(&mut self, elements: &[Option<LayoutElementId>]);
    /// These nodes' styles must be recomputed before the next paint.
    fn damage_nodes(&mut self, nodes: &[NodeId]);
    /// The page must be rebuilt from the document: boxes may have moved.
    fn relayout(&mut self);
}

/// What a key press came to, for a host that owns navigation and scrolling.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// The focused control or the focus machinery took the key.
    Consumed,
    /// Enter on a focused link: the host decides whether to follow `href`.
    FollowLink(String),
    /// Not the page's key: the host may scroll with it, or drop it.
    Unhandled,
}

/// A picker input the user activated: the embedder should open its picker over it.
#[derive(Debug, Clone)]
pub struct PickerRequest {
    pub node: NodeId,
    pub kind: PickerKind,
    /// The control's border box in viewport CSS px.
    pub anchor: Rect,
    /// Its current value, sanitised.
    pub value: String,
    pub min: Option<String>,
    pub max: Option<String>,
    pub step: Option<String>,
}

/// A textarea resize in progress: where the pointer started and the border-box size then.
#[derive(Debug, Clone, Copy)]
struct ResizeDrag {
    node: NodeId,
    start: (f64, f64),
    size: (f64, f64),
    horizontal: bool,
    vertical: bool,
}

/// The input state of one page that is not the document's own: gestures in progress and
/// what the last gesture asked of the embedder. Focus, selection, edit state, checkedness and
/// the open dropdown live on the document, where the painter reads them.
#[derive(Default)]
pub(crate) struct PageInput {
    /// A form submit triggered by the last click/key, for the tab worker to navigate.
    pending_submission: Option<Submission>,
    /// Range slider being dragged: the input and its layout element (for track geometry).
    drag_range: Option<(NodeId, LayoutElementId)>,
    /// Textarea corner being dragged.
    drag_resize: Option<ResizeDrag>,
    /// Dropdown scrollbar thumb being dragged: pointer y and `first_row` at the press.
    drag_popup_thumb: Option<(f64, usize)>,
    /// Selection being dragged out in a text control.
    drag_select: Option<(NodeId, LayoutElementId)>,
    /// Textarea scrollbar thumb being dragged: pointer y and scroll row at the press.
    drag_area_thumb: Option<(NodeId, LayoutElementId, f64, usize)>,
    /// Last press (time, position, click count) for double/triple-click detection.
    last_press: Option<(std::time::Instant, f64, f64, u8)>,
    /// Clipboard traffic towards the embedder; see `text_ui`.
    clipboard_write: Option<String>,
    paste_requested: bool,
    /// A picker input the user activated, waiting for the tab worker to tell the embedder.
    picker_request: Option<PickerRequest>,
    /// The input whose picker the embedder has open; its answers land here.
    picker_target: Option<NodeId>,
    /// Dropdown type-ahead: the letters typed so far and when the last one arrived.
    typeahead: Option<(String, std::time::Instant)>,
}

/// The DOM node and layout element under viewport point `(vp_x, vp_y)`.
pub(crate) fn hit_at(
    layer_list: Option<&LayerList>,
    (scroll_x, scroll_y): (f64, f64),
    vp_x: f64,
    vp_y: f64,
) -> (Option<NodeId>, Option<LayoutElementId>) {
    layer_list.map_or((None, None), |layer_list| {
        // find_element_at handles scroll per-layer (fixed layers ignore it).
        let Some(lei) = layer_list.find_element_at(vp_x, vp_y, scroll_x, scroll_y) else {
            return (None, None);
        };
        // A `::before`/`::after` box counts as a hit on its owner.
        let dom_node_id = layer_list
            .layout_tree
            .get_node_by_id(lei)
            .map(|el| pseudo_owner(el.dom_node_id).unwrap_or(el.dom_node_id));
        (dom_node_id, Some(lei))
    })
}

/// The layout element `node` was laid out into, if it has one.
pub(crate) fn layout_element_of(layer_list: Option<&LayerList>, node: NodeId) -> Option<LayoutElementId> {
    layer_list?
        .layout_tree
        .arena
        .iter()
        .find(|(_, el)| el.dom_node_id == node)
        .map(|(id, _)| *id)
}

/// The border box of `node`'s form control, in document coordinates.
fn control_anchor(layer_list: Option<&LayerList>, node: NodeId) -> Option<Rect> {
    layer_list?
        .layout_tree
        .arena
        .values()
        .find(|el| el.dom_node_id == node && matches!(el.context, ElementContext::FormControl(_)))
        .map(|el| el.box_model.border_box)
}

/// A press inside the bottom-right grip of a resizable textarea starts a resize.
fn resize_grip_hit(
    layer_list: Option<&LayerList>,
    (scroll_x, scroll_y): (f64, f64),
    node: NodeId,
    lei: LayoutElementId,
    vp_x: f64,
    vp_y: f64,
) -> Option<ResizeDrag> {
    let el = layer_list?.layout_tree.get_node_by_id(lei)?;
    let ElementContext::FormControl(fc) = &el.context else {
        return None;
    };
    let FormControl::TextField { resize, .. } = &fc.control else {
        return None;
    };
    if *resize == Resize::None {
        return None;
    }
    let bb = el.box_model.border_box;
    let (x, y) = (vp_x + scroll_x, vp_y + scroll_y);
    const GRIP: f64 = 16.0;
    if x < bb.x + bb.width - GRIP || y < bb.y + bb.height - GRIP {
        return None;
    }
    Some(ResizeDrag {
        node,
        start: (vp_x, vp_y),
        size: (bb.width, bb.height),
        horizontal: matches!(resize, Resize::Both | Resize::Horizontal),
        vertical: matches!(resize, Resize::Both | Resize::Vertical),
    })
}

impl PageInput {
    /// Whether the focused element is text-editable (input/textarea/contenteditable).
    pub fn focused_editable<H: InputHost>(host: &H) -> bool {
        match host.document() {
            Some(doc) => doc.focused_node().is_some_and(|id| is_text_input(&doc, id)),
            None => false,
        }
    }

    /// The focused element's link target (`<a href>`), for Enter-to-activate.
    pub fn focused_link<H: InputHost>(host: &H) -> Option<String> {
        let doc = host.document()?;
        let id = doc.focused_node()?;
        if doc.tag_name(id) == Some("a") {
            doc.attribute(id, "href").map(str::to_string)
        } else {
            None
        }
    }

    /// Move focus to `node` (`None` blurs); `visible` = show the ring.
    ///
    /// `:focus` only repaints the element itself - `:focus-within` is not implemented
    /// (`gosub_css3` matcher: "focus-within needs the focus chain; not tracked yet"), so no
    /// ancestor's styles can change. That makes this the same shape as a hover move: two
    /// elements' worth of paint damage rather than a whole-document rebuild.
    pub fn set_focus<H: InputHost>(&mut self, host: &mut H, node: Option<NodeId>, visible: bool) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let previous = doc.focused_node();
        let unchanged = previous == node && node.is_none_or(|n| doc.is_focus_visible(n) == visible);
        if unchanged {
            return false;
        }
        doc.set_focused_node(node, visible);
        let nodes: Vec<NodeId> = [previous, node].into_iter().flatten().collect();
        host.damage_nodes(&nodes);
        let ll = host.layer_list();
        let leis = [previous, node].map(|id| id.and_then(|id| layout_element_of(ll.as_deref(), id)));
        host.repaint_elements(&leis);
        true
    }

    /// Click-to-focus: the nearest focusable element under the point (via `<label>` bindings),
    /// or blur when there is none.
    pub fn focus_at<H: InputHost>(&mut self, host: &mut H, vp_x: f64, vp_y: f64) -> bool {
        let ll = host.layer_list();
        let (leaf, lei) = hit_at(ll.as_deref(), host.scroll(), vp_x, vp_y);
        let (target, visible) = match (host.document(), leaf) {
            (Some(doc), Some(leaf)) => {
                let target = focus::click_target(&doc, leaf);
                let visible = target.is_some_and(|t| focus::click_shows_ring(&doc, t));
                (target, visible)
            }
            _ => (None, false),
        };
        log::debug!("focus: click at ({vp_x}, {vp_y}) hit {leaf:?} -> focus {target:?} (ring: {visible})");
        let changed = self.set_focus(host, target, visible);
        // A click straight into a text control also puts the caret where it landed.
        let placed = match (target, lei) {
            (Some(t), Some(lei)) if leaf == Some(t) => self.place_caret(host, t, lei, vp_x, vp_y),
            _ => false,
        };
        changed || placed
    }

    /// Click activation of what's under the point: picks a dropdown row, opens/closes a
    /// `<select>`, toggles a checkbox / selects a radio. Any click closes an open dropdown.
    pub fn activate_at<H: InputHost>(&mut self, host: &mut H, vp_x: f64, vp_y: f64) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let ll = host.layer_list();
        let (leaf, lei) = hit_at(ll.as_deref(), host.scroll(), vp_x, vp_y);

        if doc.open_select().is_some() {
            return self.popup_press(host, lei, vp_x, vp_y);
        }

        let Some(target) = leaf.and_then(|l| focus::click_target(&doc, l)) else {
            return false;
        };
        // Pressing on a slider's own box jumps the thumb there and starts a drag.
        if leaf == Some(target) && edit::range_params(&doc, target).is_some() {
            if let Some(lei) = lei {
                self.drag_range = Some((target, lei));
                return self.drag_to(host, vp_x);
            }
        }
        // Pressing a textarea's grip corner starts a resize.
        if leaf == Some(target) {
            if let Some(drag) =
                lei.and_then(|lei| resize_grip_hit(ll.as_deref(), host.scroll(), target, lei, vp_x, vp_y))
            {
                self.drag_resize = Some(drag);
                return true;
            }
        }
        // Pressing a text control's own box: scrollbar, multi-click selection, drag start.
        if leaf == Some(target) {
            if let Some(lei) = lei.filter(|_| edit::text_entry_kind(&doc, target).is_some()) {
                return self.text_press(host, target, lei, vp_x, vp_y);
            }
        }
        match form::button_kind(&doc, target) {
            Some(false) => return self.submit(host, target, Some(target)),
            Some(true) => return Self::reset_form(host, target),
            None => {}
        }
        if edit::is_select(&doc, target) {
            return Self::open_select_popup(host, target);
        }
        if let Some(kind) = edit::picker_kind(&doc, target) {
            return self.request_picker(host, target, kind);
        }
        Self::toggle_control(host, target)
    }

    /// Ask the embedder to open its picker for `node`. Nothing is drawn: the request is parked
    /// for the tab worker to emit, and the answers arrive through [`Self::set_picker_value`].
    fn request_picker<H: InputHost>(&mut self, host: &mut H, node: NodeId, kind: PickerKind) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let anchor = control_anchor(host.layer_list().as_deref(), node).unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
        // Viewport coordinates, like the pointer events the shell sends: it is placing a
        // window over the control, not over the document.
        let (scroll_x, scroll_y) = host.scroll();
        let anchor = Rect::new(anchor.x - scroll_x, anchor.y - scroll_y, anchor.width, anchor.height);
        let (min, max, step) = edit::picker_bounds(&doc, node);
        self.picker_request = Some(PickerRequest {
            node,
            kind,
            anchor,
            value: edit::picker_value(&doc, node, kind),
            min,
            max,
            step,
        });
        self.picker_target = Some(node);
        true
    }

    /// The picker input activated since the last call, for the tab worker to pass on.
    pub fn take_picker_request(&mut self) -> Option<PickerRequest> {
        self.picker_request.take()
    }

    /// The embedder's picker moved to `value`: store it on the input that asked, sanitised for
    /// its kind. False when nothing changed, or no picker is open.
    pub fn set_picker_value<H: InputHost>(&mut self, host: &mut H, value: &str) -> bool {
        let (Some(node), Some(doc)) = (self.picker_target, host.document()) else {
            return false;
        };
        let Some(kind) = edit::picker_kind(&doc, node) else {
            return false;
        };
        let next = edit::sanitize_picker_value(kind, value);
        if edit::picker_value(&doc, node, kind) == next && doc.control_edit_state(node).is_some() {
            return false;
        }
        doc.set_control_edit_state(node, Some(gosub_interface::document::ControlEditState::new(next, 0)));
        Self::request_repaint(host, node);
        true
    }

    /// The embedder closed its picker; further answers are ignored until the next request.
    pub fn end_picker(&mut self) {
        self.picker_target = None;
    }

    /// Forget a picker request and its target: the document they named is gone, and an answer
    /// for the old picker must not land on whatever node the new document gave the same id to.
    pub fn forget_picker(&mut self) {
        self.picker_request = None;
        self.picker_target = None;
    }

    /// Pointer moved with the button held: follow a slider drag (paint-only) or a textarea
    /// resize (re-layout).
    pub fn drag_move<H: InputHost>(&mut self, host: &mut H, vp_x: f64, vp_y: f64) -> bool {
        if self.drag_range.is_some() {
            return self.drag_to(host, vp_x);
        }
        if let Some((start_y, start_first)) = self.drag_popup_thumb {
            return self.popup_thumb_drag_to(host, start_y, start_first, vp_y);
        }
        if self.drag_select.is_some() {
            return self.drag_select_to(host, vp_x, vp_y);
        }
        if self.drag_area_thumb.is_some() {
            return self.area_thumb_drag_to(host, vp_y);
        }
        let (Some(drag), Some(doc)) = (self.drag_resize, host.document()) else {
            return false;
        };
        let (mut w, mut h) = drag.size;
        if drag.horizontal {
            w = (drag.size.0 + vp_x - drag.start.0).max(40.0);
        }
        if drag.vertical {
            h = (drag.size.1 + vp_y - drag.start.1).max(30.0);
        }
        if doc.control_size(drag.node) == Some((w, h)) {
            return false;
        }
        doc.set_control_size(drag.node, Some((w, h)));
        host.relayout();
        true
    }

    pub fn end_drag(&mut self) {
        self.drag_range = None;
        self.drag_resize = None;
        self.drag_popup_thumb = None;
        self.drag_select = None;
        self.drag_area_thumb = None;
    }

    pub fn is_resizing(&self) -> bool {
        self.drag_resize.is_some()
    }

    /// Set the dragged slider from a viewport x, mapping the thumb's travel across the content
    /// box the same way the painter does (thumb diameter = min(12, height)).
    fn drag_to<H: InputHost>(&mut self, host: &mut H, vp_x: f64) -> bool {
        let (Some((node, lei)), Some(doc)) = (self.drag_range, host.document()) else {
            return false;
        };
        let Some((min, max, step)) = edit::range_params(&doc, node) else {
            return false;
        };
        let Some(cb) = host
            .layer_list()
            .and_then(|ll| ll.layout_tree.get_node_by_id(lei).map(|el| el.box_model.content_box))
        else {
            return false;
        };
        let d = 12.0_f64.min(cb.height);
        let travel = (cb.width - d).max(1.0);
        let (scroll_x, _) = host.scroll();
        let fraction = ((vp_x + scroll_x - cb.x - d / 2.0) / travel).clamp(0.0, 1.0);
        let value = edit::range_snap(min, max, step, min + fraction * (max - min));
        Self::set_range_value(host, node, value)
    }

    fn set_range_value<H: InputHost>(host: &mut H, node: NodeId, value: f64) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let (min, max, _) = edit::range_params(&doc, node).unwrap_or((0.0, 100.0, 1.0));
        if edit::range_value(&doc, node, min, max) == value && doc.control_edit_state(node).is_some() {
            return false;
        }
        doc.set_control_edit_state(
            node,
            Some(gosub_interface::document::ControlEditState::new(
                edit::format_number(value),
                0,
            )),
        );
        Self::request_repaint(host, node);
        true
    }

    /// Keyboard on a focused slider: arrows step, PageUp/Down jump 10 steps, Home/End go to the
    /// ends.
    fn range_key<H: InputHost>(host: &mut H, node: NodeId, key: &str) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let Some((min, max, step)) = edit::range_params(&doc, node) else {
            return false;
        };
        let cur = edit::range_value(&doc, node, min, max);
        let target = match key {
            "ArrowRight" | "ArrowUp" => cur + step,
            "ArrowLeft" | "ArrowDown" => cur - step,
            "PageUp" => cur + step * 10.0,
            "PageDown" => cur - step * 10.0,
            "Home" => min,
            "End" => max,
            _ => return false,
        };
        Self::set_range_value(host, node, edit::range_snap(min, max, step, target));
        true
    }

    /// The submission the last click/Enter asked for, if any (consumed).
    pub fn take_submission(&mut self) -> Option<Submission> {
        self.pending_submission.take()
    }

    /// Submit the form owning `control` (a submit button, or a text field on Enter). Nothing
    /// happens outside a form or without a document URL to resolve against.
    fn submit<H: InputHost>(&mut self, host: &mut H, control: NodeId, submitter: Option<NodeId>) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let Some(form) = form::form_owner(&doc, control) else {
            return false;
        };
        let Some(base) = doc.url() else {
            return false;
        };
        self.pending_submission = form::submission(&doc, form, submitter, &base);
        self.pending_submission.is_some()
    }

    /// Reset button: forget everything typed/toggled/picked in its form.
    fn reset_form<H: InputHost>(host: &mut H, button: NodeId) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let Some(form) = form::form_owner(&doc, button) else {
            return false;
        };
        for id in form::controls(&doc, form) {
            doc.set_control_edit_state(id, None);
            doc.set_checked(id, None);
            doc.set_selected_option(id, None);
        }
        host.relayout();
        true
    }

    /// Enter in a single-line text field: implicit submission through the form's first submit
    /// button (or without one when the form has a single text field).
    fn implicit_submit<H: InputHost>(&mut self, host: &mut H, field: NodeId) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let Some(form) = form::form_owner(&doc, field) else {
            return false;
        };
        match form::default_submitter(&doc, form) {
            Some(submitter) => self.submit(host, field, submitter),
            None => false,
        }
    }

    fn toggle_control<H: InputHost>(host: &mut H, node: NodeId) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let changes = edit::toggle(&doc, node);
        if changes.is_empty() {
            return false;
        }
        for (n, checked) in changes {
            doc.set_checked(n, Some(checked));
        }
        // `:checked` rules may restyle siblings and change layout.
        host.relayout();
        true
    }

    /// Key press for the focused control: text editing, or Space toggling a checkbox/radio.
    /// Returns whether the key was consumed. Ctrl/Meta chords are left alone.
    pub fn edit_key<H: InputHost>(
        &mut self,
        host: &mut H,
        key: &str,
        ctrl_or_meta: bool,
        alt: bool,
        shift: bool,
    ) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let Some(node) = doc.focused_node() else {
            return false;
        };
        if key == " " && !ctrl_or_meta && edit::toggle_kind(&doc, node).is_some() {
            return Self::toggle_control(host, node);
        }
        if edit::is_select(&doc, node) && !ctrl_or_meta {
            return self.select_key(host, node, key, alt);
        }
        if edit::range_params(&doc, node).is_some() && !ctrl_or_meta {
            return Self::range_key(host, node, key);
        }
        if let Some(kind) = edit::picker_kind(&doc, node).filter(|_| !ctrl_or_meta && matches!(key, "Enter" | " ")) {
            return self.request_picker(host, node, kind);
        }
        if matches!(key, "Enter" | " ") && !ctrl_or_meta {
            match form::button_kind(&doc, node) {
                Some(false) => return self.submit(host, node, Some(node)),
                Some(true) => return Self::reset_form(host, node),
                None => {}
            }
        }
        let Some(multiline) = edit::text_entry_kind(&doc, node) else {
            return false;
        };
        if key == "Enter" && !multiline && !ctrl_or_meta {
            return self.implicit_submit(host, node);
        }
        if ctrl_or_meta {
            let masked = doc
                .attribute(node, "type")
                .is_some_and(|t| t.eq_ignore_ascii_case("password"));
            if let Some(handled) = self.clipboard_key(host, node, key, masked) {
                return handled;
            }
        } else if multiline {
            if let Some(handled) = self.row_key(host, node, key, shift) {
                return handled;
            }
        }
        let Some(action) = edit::action_for_key(key, multiline, ctrl_or_meta, shift) else {
            return false;
        };
        self.apply_edit(host, node, &action)
    }

    /// Committed text (IME / `TextInput`) into the focused text control.
    pub fn insert_text<H: InputHost>(&mut self, host: &mut H, text: &str) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let Some(node) = doc.focused_node() else {
            return false;
        };
        if edit::text_entry_kind(&doc, node).is_none() || text.is_empty() {
            return false;
        }
        self.apply_edit(host, node, &edit::EditAction::Insert(text.to_string()))
    }

    /// Returns whether the control changed. The box doesn't depend on the value and the painter
    /// reads it live, so this is paint-only.
    fn apply_edit<H: InputHost>(&mut self, host: &mut H, node: NodeId, action: &edit::EditAction) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        let filtered;
        let action = match action {
            edit::EditAction::Insert(text) => {
                filtered = edit::EditAction::Insert(edit::filter_insert(&doc, node, text));
                if matches!(&filtered, edit::EditAction::Insert(t) if t.is_empty()) {
                    return false;
                }
                &filtered
            }
            other => other,
        };
        let mut state = Self::edit_state(host, node);
        if !edit::apply(&mut state, action) {
            return false;
        }
        self.commit_edit_state(host, node, state);
        true
    }

    /// Repaint the tiles under `node` only; full render if it has no layout element yet.
    fn request_repaint<H: InputHost>(host: &mut H, node: NodeId) {
        match layout_element_of(host.layer_list().as_deref(), node) {
            Some(lei) => host.repaint_elements(&[Some(lei)]),
            None => host.relayout(),
        }
    }

    /// Tab / Shift+Tab: next/previous element in tab order, wrapping.
    pub fn focus_step<H: InputHost>(&mut self, host: &mut H, backwards: bool) -> bool {
        let Some(doc) = host.document() else {
            return false;
        };
        if doc.open_select().is_some() {
            Self::close_select_popup(host);
        }
        // Only elements with a box are reachable by keyboard; no render yet = every focusable.
        let rendered: Option<std::collections::HashSet<NodeId>> = host
            .layer_list()
            .map(|ll| ll.layout_tree.arena.values().map(|el| el.dom_node_id).collect());
        let order = focus::tab_order(&doc, rendered.as_ref());
        if order.is_empty() {
            return self.set_focus(host, None, false);
        }
        let current = doc.focused_node().and_then(|f| order.iter().position(|&n| n == f));
        let next = match (current, backwards) {
            (Some(i), false) => (i + 1) % order.len(),
            (Some(i), true) => (i + order.len() - 1) % order.len(),
            (None, false) => 0,
            (None, true) => order.len() - 1,
        };
        let changed = self.set_focus(host, Some(order[next]), true);
        if changed {
            Self::select_all_on_focus(host, order[next]);
        }
        changed
    }

    /// A key press, as the tab worker dispatches it: the focused control first, then
    /// Tab and Shift+Tab for focus traversal, Escape to blur, Enter on a focused link.
    /// Arrow and page keys with nothing editable focused are the host's to scroll with.
    pub fn key_down<H: InputHost>(&mut self, host: &mut H, key: &str, modifiers: Modifiers) -> KeyOutcome {
        let shift = modifiers.contains(Modifiers::SHIFT);
        if key != "Tab" {
            let chord = modifiers.intersects(Modifiers::CONTROL | Modifiers::META);
            let alt = modifiers.contains(Modifiers::ALT);
            if self.edit_key(host, key, chord, alt, shift) {
                return KeyOutcome::Consumed;
            }
        }
        match key {
            "Tab" => {
                self.focus_step(host, shift);
                KeyOutcome::Consumed
            }
            "Escape" => {
                if self.set_focus(host, None, false) {
                    KeyOutcome::Consumed
                } else {
                    KeyOutcome::Unhandled
                }
            }
            "Enter" => match Self::focused_link(host) {
                Some(href) => KeyOutcome::FollowLink(href),
                None => KeyOutcome::Unhandled,
            },
            _ => KeyOutcome::Unhandled,
        }
    }

    /// The window lost focus: an open dropdown closes, focus clears, gestures end.
    /// Returns whether the page changed.
    pub fn blur<H: InputHost>(&mut self, host: &mut H) -> bool {
        let mut changed = false;
        if host.document().is_some_and(|doc| doc.open_select().is_some()) {
            Self::close_select_popup(host);
            changed = true;
        }
        changed |= self.set_focus(host, None, false);
        self.end_drag();
        changed
    }

    /// Whether a gesture holds the pointer: a slider thumb, a textarea grip or
    /// scrollbar, a dropdown scrollbar, or a selection being dragged out.
    pub fn has_capture(&self) -> bool {
        self.drag_range.is_some()
            || self.drag_resize.is_some()
            || self.drag_popup_thumb.is_some()
            || self.drag_select.is_some()
            || self.drag_area_thumb.is_some()
    }
}
