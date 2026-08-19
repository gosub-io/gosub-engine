//! Form interaction tests: drive a `BrowsingContext` the way the tab worker does (hit-test,
//! focus, activate, keys, drags) and assert the state it leaves on the document. Runs the real
//! pipeline in-process; coordinates come from the layout tree, never from pixels.

use super::*;
use crate::engine::settings_store;
use crate::html::DefaultRenderConfig;
use gosub_css3::system::Css3System;
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::texture::TextureId;
use gosub_render_pipeline::common::texture_store::TextureStore;
use gosub_render_pipeline::layouter::ElementContext;
use gosub_render_pipeline::render::backend::PixelFormat;
use gosub_render_pipeline::tiler::Tile;

struct StubRasterizer;

impl Rasterable for StubRasterizer {
    fn rasterize(&self, tile: &Tile, store: &mut TextureStore, _media: &MediaStore) -> Option<TextureId> {
        let (w, h) = (tile.rect.width as usize, tile.rect.height as usize);
        Some(store.add(w, h, vec![0u8; w * h * 4], PixelFormat::PreMulArgb32))
    }
}

type Ctx = BrowsingContext<DefaultRenderConfig>;

/// A laid-out context for `body_html` at http://test.local/page.html.
fn page(body_html: &str) -> Ctx {
    let mut ctx: Ctx = BrowsingContext::new(settings_store::default_config());
    ctx.set_rasterizer(Box::new(StubRasterizer), RasterStrategy::ParallelCached);
    ctx.set_viewport(Viewport {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    });
    let html = format!("<!DOCTYPE html><html><body style=\"margin:0;padding:20px\">{body_html}</body></html>");
    let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(&html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    doc.url = Some(url::Url::parse("http://test.local/page.html").unwrap_or_else(|_| unreachable!()));
    ctx.set_document(Arc::new(doc));
    ctx.rebuild_pipeline_cache_if_needed();
    ctx
}

fn render(ctx: &mut Ctx) {
    ctx.rebuild_pipeline_cache_if_needed();
}

fn doc(ctx: &Ctx) -> Arc<EngineDocument<DefaultRenderConfig>> {
    ctx.document
        .clone()
        .unwrap_or_else(|| unreachable!("page() set a document"))
}

fn by_id(ctx: &Ctx, id: &str) -> NodeId {
    doc(ctx)
        .node_by_named_id(id)
        .unwrap_or_else(|| panic!("no element with id {id}"))
}

/// Centre of the element's border box (viewport px; the test pages don't scroll).
fn center(ctx: &Ctx, node: NodeId) -> (f64, f64) {
    let ll = ctx.active_layer_list().unwrap_or_else(|| unreachable!("rendered"));
    let el = ll
        .layout_tree
        .arena
        .values()
        .filter(|el| el.dom_node_id == node && !matches!(el.context, ElementContext::SelectPopup(_)))
        .min_by_key(|el| el.id.as_u64())
        .unwrap_or_else(|| panic!("{node:?} has no layout box"));
    let b = el.box_model.border_box;
    (b.x + b.width / 2.0, b.y + b.height / 2.0)
}

fn border_box(ctx: &Ctx, node: NodeId) -> gosub_render_pipeline::common::geo::Rect {
    let ll = ctx.active_layer_list().unwrap_or_else(|| unreachable!("rendered"));
    ll.layout_tree
        .arena
        .values()
        .filter(|el| el.dom_node_id == node && !matches!(el.context, ElementContext::SelectPopup(_)))
        .min_by_key(|el| el.id.as_u64())
        .map(|el| el.box_model.border_box)
        .unwrap_or_else(|| panic!("{node:?} has no layout box"))
}

/// What the tab worker does for a left click at viewport `(x, y)`.
fn click_at(ctx: &mut Ctx, x: f64, y: f64) {
    ctx.update_hover(x, y);
    ctx.focus_at(x, y);
    ctx.activate_at(x, y);
    ctx.end_drag();
    render(ctx);
}

fn click(ctx: &mut Ctx, id: &str) {
    let (x, y) = center(ctx, by_id(ctx, id));
    click_at(ctx, x, y);
}

fn key(ctx: &mut Ctx, k: &str) {
    if k == "Tab" {
        ctx.focus_step(false);
    } else if k == "ShiftTab" {
        ctx.focus_step(true);
    } else {
        ctx.edit_key(k, false, false);
    }
    render(ctx);
}

fn type_text(ctx: &mut Ctx, text: &str) {
    for ch in text.chars() {
        key(ctx, &ch.to_string());
    }
}

fn value(ctx: &Ctx, id: &str) -> String {
    let d = doc(ctx);
    let node = by_id(ctx, id);
    d.control_edit_state(node)
        .map(|s| s.value)
        .unwrap_or_else(|| crate::engine::edit::initial_value(&d, node))
}

fn focused_id(ctx: &Ctx) -> Option<String> {
    let d = doc(ctx);
    ctx.focused_node()
        .and_then(|n| d.attribute(n, "id").map(str::to_string))
}

// ── focus ─────────────────────────────────────────────────────────────────────

#[test]
fn tab_walks_focusables_in_order_and_wraps() {
    let mut ctx = page(
        r#"<input id="a" type="text">
           <button id="b">B</button>
           <a id="c" href="/x">link</a>
           <button id="d" tabindex="-1">skipped</button>
           <input id="e" type="checkbox">
           <input id="dis" type="text" disabled>"#,
    );
    let mut seen = Vec::new();
    for _ in 0..5 {
        key(&mut ctx, "Tab");
        seen.push(focused_id(&ctx).unwrap_or_default());
    }
    assert_eq!(
        seen,
        ["a", "b", "c", "e", "a"],
        "DOM order, tabindex=-1 and disabled skipped, wraps"
    );
    key(&mut ctx, "ShiftTab");
    assert_eq!(focused_id(&ctx).as_deref(), Some("e"), "Shift+Tab wraps backwards");
}

#[test]
fn positive_tabindex_comes_first() {
    let mut ctx = page(r#"<button id="x">x</button><button id="y" tabindex="1">y</button>"#);
    key(&mut ctx, "Tab");
    assert_eq!(focused_id(&ctx).as_deref(), Some("y"));
    key(&mut ctx, "Tab");
    assert_eq!(focused_id(&ctx).as_deref(), Some("x"));
}

#[test]
fn click_focuses_and_clicking_elsewhere_blurs() {
    let mut ctx = page(r#"<input id="t" type="text"><p id="p" style="margin-top:60px">plain text</p>"#);
    click(&mut ctx, "t");
    assert_eq!(focused_id(&ctx).as_deref(), Some("t"));
    assert!(
        doc(&ctx).is_focus_visible(by_id(&ctx, "t")),
        "text fields show the ring on click"
    );
    click(&mut ctx, "p");
    assert_eq!(ctx.focused_node(), None);
}

#[test]
fn label_click_reaches_its_control() {
    let mut ctx = page(
        r#"<label id="l1"><input id="c1" type="checkbox"> by wrapping</label>
           <label id="l2" for="c2">by for</label> <input id="c2" type="checkbox">"#,
    );
    click(&mut ctx, "l1");
    assert!(doc(&ctx).is_checked(by_id(&ctx, "c1")));
    assert_eq!(focused_id(&ctx).as_deref(), Some("c1"));
    click(&mut ctx, "l2");
    assert!(doc(&ctx).is_checked(by_id(&ctx, "c2")));
}

// ── text entry ────────────────────────────────────────────────────────────────

#[test]
fn typing_edits_value_and_caret() {
    let mut ctx = page(r#"<input id="t" type="text" value="hi">"#);
    click(&mut ctx, "t");
    type_text(&mut ctx, "ab");
    assert_eq!(value(&ctx, "t"), "hiab");
    key(&mut ctx, "Backspace");
    key(&mut ctx, "ArrowLeft");
    type_text(&mut ctx, "X");
    assert_eq!(value(&ctx, "t"), "hiXa");
    key(&mut ctx, "Home");
    key(&mut ctx, "Delete");
    assert_eq!(value(&ctx, "t"), "iXa");
    key(&mut ctx, "End");
    type_text(&mut ctx, "!");
    assert_eq!(value(&ctx, "t"), "iXa!");
}

#[test]
fn number_field_refuses_letters() {
    let mut ctx = page(r#"<input id="n" type="number" value="4">"#);
    click(&mut ctx, "n");
    type_text(&mut ctx, "2a.5x-e");
    assert_eq!(value(&ctx, "n"), "42.5-e");
}

#[test]
fn textarea_takes_newlines_and_enter_in_input_submits() {
    let mut ctx = page(
        r#"<form action="/go"><textarea id="ta">one</textarea><input id="q" name="q" value="x"><button>ok</button></form>"#,
    );
    click(&mut ctx, "ta");
    key(&mut ctx, "Enter");
    type_text(&mut ctx, "two");
    assert_eq!(value(&ctx, "ta"), "one\ntwo");
    assert!(ctx.take_submission().is_none(), "Enter in a textarea never submits");
    click(&mut ctx, "q");
    key(&mut ctx, "Enter");
    let sub = ctx
        .take_submission()
        .unwrap_or_else(|| panic!("Enter in a text field submits"));
    assert_eq!(sub.url.as_str(), "http://test.local/go?q=x");
}

// ── checkboxes & radios ───────────────────────────────────────────────────────

#[test]
fn checkbox_toggles_and_radio_groups_are_exclusive() {
    let mut ctx = page(
        r#"<input id="c" type="checkbox" checked>
           <input id="r1" type="radio" name="g" checked><input id="r2" type="radio" name="g"><input id="r3" type="radio" name="g" disabled>"#,
    );
    let d = doc(&ctx);
    click(&mut ctx, "c");
    assert!(!d.is_checked(by_id(&ctx, "c")));
    key(&mut ctx, " ");
    assert!(d.is_checked(by_id(&ctx, "c")), "Space toggles the focused checkbox");

    click(&mut ctx, "r2");
    assert!(d.is_checked(by_id(&ctx, "r2")));
    assert!(!d.is_checked(by_id(&ctx, "r1")));
    click(&mut ctx, "r2");
    assert!(
        d.is_checked(by_id(&ctx, "r2")),
        "clicking a checked radio keeps it checked"
    );
    click(&mut ctx, "r3");
    assert!(!d.is_checked(by_id(&ctx, "r3")), "disabled radio ignores clicks");
}

// ── select ────────────────────────────────────────────────────────────────────

const SELECT: &str = r#"<select id="s">
    <option id="o1">Apple</option>
    <optgroup label="Berries"><option id="o2" disabled>Blueberry</option><option id="o3" selected>Cherry</option></optgroup>
    <option id="o4">Date</option>
</select><p style="margin-top:400px">filler</p>"#;

fn selected(ctx: &Ctx, select: &str) -> Option<String> {
    let d = doc(ctx);
    d.selected_option(by_id(ctx, select))
        .and_then(|o| d.attribute(o, "id").map(str::to_string))
}

#[test]
fn select_opens_navigates_commits_and_escapes() {
    let mut ctx = page(SELECT);
    let d = doc(&ctx);
    assert_eq!(selected(&ctx, "s").as_deref(), Some("o3"));

    click(&mut ctx, "s");
    let open = d.open_select().unwrap_or_else(|| panic!("click opens the dropdown"));
    // Popup rows: Apple(0) Berries(1) Blueberry(2) Cherry(3) Date(4); the choice is active.
    assert_eq!(open.active, Some(3));

    key(&mut ctx, "ArrowUp");
    assert_eq!(
        d.open_select().and_then(|o| o.active),
        Some(0),
        "skips the disabled option and the group label"
    );
    key(&mut ctx, "ArrowDown");
    key(&mut ctx, "ArrowDown");
    assert_eq!(d.open_select().and_then(|o| o.active), Some(4));
    assert_eq!(
        selected(&ctx, "s").as_deref(),
        Some("o3"),
        "arrows while open don't commit"
    );

    key(&mut ctx, "Escape");
    assert!(d.open_select().is_none());
    assert_eq!(selected(&ctx, "s").as_deref(), Some("o3"), "Escape changes nothing");

    key(&mut ctx, "Enter");
    assert!(d.open_select().is_some(), "Enter reopens");
    key(&mut ctx, "ArrowDown");
    key(&mut ctx, "Enter");
    assert!(d.open_select().is_none());
    assert_eq!(
        selected(&ctx, "s").as_deref(),
        Some("o4"),
        "Enter commits the active row"
    );
}

#[test]
fn select_closed_keys_change_selection_and_typeahead_matches() {
    let mut ctx = page(SELECT);
    click(&mut ctx, "s");
    key(&mut ctx, "Escape");
    key(&mut ctx, "Home");
    assert_eq!(selected(&ctx, "s").as_deref(), Some("o1"));
    key(&mut ctx, "End");
    assert_eq!(selected(&ctx, "s").as_deref(), Some("o4"));
    type_text(&mut ctx, "ch");
    assert_eq!(
        selected(&ctx, "s").as_deref(),
        Some("o3"),
        "type-ahead picks the first 'ch…'"
    );
}

#[test]
fn select_row_click_picks_and_outside_click_closes() {
    let mut ctx = page(SELECT);
    let d = doc(&ctx);
    click(&mut ctx, "s");
    // Row 4 ("Date") sits below the popup's top padding at 4 × row_height.
    let (popup_box, row_h) = {
        let ll = ctx.active_layer_list().unwrap_or_else(|| unreachable!());
        let el = ll
            .layout_tree
            .get_node_by_id(ll.layout_tree.popup.unwrap_or_else(|| unreachable!("popup exists")))
            .unwrap_or_else(|| unreachable!());
        let ElementContext::SelectPopup(p) = &el.context else {
            unreachable!()
        };
        (el.box_model.content_box, p.row_height)
    };
    click_at(&mut ctx, popup_box.x + 20.0, popup_box.y + 4.0 * row_h + row_h / 2.0);
    assert!(d.open_select().is_none());
    assert_eq!(selected(&ctx, "s").as_deref(), Some("o4"));

    click(&mut ctx, "s");
    assert!(d.open_select().is_some());
    click_at(&mut ctx, 700.0, 500.0);
    assert!(d.open_select().is_none(), "click outside closes");
    assert_eq!(selected(&ctx, "s").as_deref(), Some("o4"), "…without changing");
}

// ── range ─────────────────────────────────────────────────────────────────────

#[test]
fn range_keys_and_drag() {
    let mut ctx = page(r#"<input id="r" type="range" min="0" max="100" step="5" value="50">"#);
    click(&mut ctx, "r");
    key(&mut ctx, "ArrowRight");
    assert_eq!(value(&ctx, "r"), "55");
    key(&mut ctx, "End");
    assert_eq!(value(&ctx, "r"), "100");
    key(&mut ctx, "PageDown");
    assert_eq!(value(&ctx, "r"), "50");

    // Press on the left end and drag to the right end.
    let b = border_box(&ctx, by_id(&ctx, "r"));
    let y = b.y + b.height / 2.0;
    ctx.focus_at(b.x + 1.0, y);
    ctx.activate_at(b.x + 1.0, y);
    assert_eq!(value(&ctx, "r"), "0", "press jumps the thumb");
    ctx.drag_move(b.x + b.width - 1.0, y);
    ctx.end_drag();
    assert_eq!(value(&ctx, "r"), "100");
}

// ── submit / reset ────────────────────────────────────────────────────────────

#[test]
fn submit_encodes_live_state_and_reset_restores_it() {
    let mut ctx = page(
        r#"<form id="f" action="/echo" method="post">
             <input id="t" name="t" value="a b">
             <input id="c" name="c" type="checkbox" value="on" checked>
             <input id="u" name="u" type="checkbox" value="on">
             <input name="r" type="radio" value="1" checked><input id="r2" name="r" type="radio" value="2">
             <select name="s"><option value="x">X</option><option value="y" selected>Y</option></select>
             <input name="h" type="hidden" value="h&v">
             <input name="dis" value="no" disabled>
             <button id="go" name="btn" value="go">Go</button>
             <button id="other" name="btn" value="other">Other</button>
             <button id="reset" type="reset">Reset</button>
           </form>"#,
    );
    click(&mut ctx, "t");
    type_text(&mut ctx, "c");
    click(&mut ctx, "c");
    click(&mut ctx, "r2");
    click(&mut ctx, "go");
    let sub = ctx.take_submission().unwrap_or_else(|| panic!("submit button submits"));
    assert!(sub.post);
    assert_eq!(sub.url.as_str(), "http://test.local/echo");
    assert_eq!(sub.body.as_deref(), Some("t=a+bc&r=2&s=y&h=h%26v&btn=go"));

    click(&mut ctx, "reset");
    assert_eq!(value(&ctx, "t"), "a b");
    assert!(doc(&ctx).is_checked(by_id(&ctx, "c")));
    assert!(!doc(&ctx).is_checked(by_id(&ctx, "r2")));
    assert!(ctx.take_submission().is_none());
}

#[test]
fn get_submit_replaces_the_query() {
    let mut ctx = page(r#"<form action="/s?old=1"><input name="q" value="v"><button id="b">go</button></form>"#);
    click(&mut ctx, "b");
    let sub = ctx.take_submission().unwrap_or_else(|| panic!("submits"));
    assert!(!sub.post);
    assert_eq!(sub.url.as_str(), "http://test.local/s?q=v");
}

// ── textarea resize ───────────────────────────────────────────────────────────

#[test]
fn textarea_grip_drag_resizes() {
    let mut ctx = page(r#"<textarea id="ta" rows="3" cols="20">x</textarea>"#);
    let before = border_box(&ctx, by_id(&ctx, "ta"));
    let (gx, gy) = (before.x + before.width - 4.0, before.y + before.height - 4.0);
    ctx.focus_at(gx, gy);
    ctx.activate_at(gx, gy);
    ctx.drag_move(gx + 50.0, gy + 30.0);
    ctx.end_drag();
    render(&mut ctx);
    let after = border_box(&ctx, by_id(&ctx, "ta"));
    assert!(
        (after.width - (before.width + 50.0)).abs() < 1.0,
        "{before:?} -> {after:?}"
    );
    assert!(
        (after.height - (before.height + 30.0)).abs() < 1.0,
        "{before:?} -> {after:?}"
    );
}
