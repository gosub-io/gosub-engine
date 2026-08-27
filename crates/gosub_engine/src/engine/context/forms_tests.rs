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
use gosub_render_pipeline::painter::text_field;
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
    chord(ctx, k, false, false);
}

/// A key with Ctrl (`ctrl`) and/or Shift held.
fn chord(ctx: &mut Ctx, k: &str, ctrl: bool, shift: bool) {
    if k == "Tab" {
        ctx.focus_step(shift);
    } else if k == "ShiftTab" {
        ctx.focus_step(true);
    } else {
        ctx.edit_key(k, ctrl, false, shift);
    }
    render(ctx);
}

fn selection(ctx: &Ctx, id: &str) -> Option<(usize, usize)> {
    doc(ctx).control_edit_state(by_id(ctx, id)).and_then(|s| s.selection())
}

fn caret(ctx: &Ctx, id: &str) -> Option<usize> {
    doc(ctx).control_edit_state(by_id(ctx, id)).map(|s| s.caret)
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
fn contenteditable_false_is_not_focusable() {
    let mut ctx = page(
        r#"<div id="on" contenteditable>on</div>
           <div id="empty" contenteditable="">empty</div>
           <div id="off" contenteditable="false">off</div>
           <div id="plain">plain</div>"#,
    );
    let mut seen = Vec::new();
    for _ in 0..3 {
        key(&mut ctx, "Tab");
        seen.push(focused_id(&ctx).unwrap_or_default());
    }
    assert_eq!(
        seen,
        ["on", "empty", "on"],
        "contenteditable=false opts out, like the empty and `true` values opt in"
    );
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
fn select_space_opens_and_commits() {
    let mut ctx = page(SELECT);
    let d = doc(&ctx);
    click(&mut ctx, "s");
    key(&mut ctx, "Escape");
    assert!(d.open_select().is_none());

    key(&mut ctx, " ");
    assert!(d.open_select().is_some(), "Space opens a closed select");

    key(&mut ctx, "ArrowDown");
    key(&mut ctx, " ");
    assert!(d.open_select().is_none());
    assert_eq!(
        selected(&ctx, "s").as_deref(),
        Some("o4"),
        "Space commits the active row"
    );
}

#[test]
fn select_space_continues_an_active_typeahead_prefix() {
    let mut ctx = page(
        r#"<select id="s">
        <option id="o1">Cherry Pie</option>
        <option id="o2">Cherry Tart</option>
    </select>"#,
    );
    click(&mut ctx, "s");
    key(&mut ctx, "Escape");
    // "cherry t" must stay one prefix: the space extends it rather than committing.
    type_text(&mut ctx, "cherry t");
    assert_eq!(selected(&ctx, "s").as_deref(), Some("o2"));
    assert!(
        doc(&ctx).open_select().is_none(),
        "the space typed ahead, it did not open"
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

// ── caret placement ───────────────────────────────────────────────────────────

/// Content box and font of a text control's layout element.
fn text_box(
    ctx: &Ctx,
    node: NodeId,
) -> (
    gosub_render_pipeline::common::geo::Rect,
    gosub_render_pipeline::common::font::FontInfo,
) {
    let ll = ctx.active_layer_list().unwrap_or_else(|| unreachable!("rendered"));
    let el = ll
        .layout_tree
        .arena
        .values()
        .find(|el| el.dom_node_id == node && matches!(el.context, ElementContext::FormControl(_)))
        .unwrap_or_else(|| panic!("{node:?} is not a form control"));
    let ElementContext::FormControl(fc) = &el.context else {
        unreachable!()
    };
    (el.box_model.content_box, fc.font_info.clone())
}

#[test]
fn click_places_the_caret_in_a_text_field() {
    use gosub_render_pipeline::painter::text_field;
    let mut ctx = page(r#"<input id="t" type="text" value="hello world" size="30">"#);
    let node = by_id(&ctx, "t");
    let (content, font) = text_box(&ctx, node);
    let x_after_hello = {
        let fs = ctx.font_system();
        let w = text_field::width(&mut *fs.lock(), "hello", &font);
        content.x + text_field::inset_x(content.width) + w + 1.0
    };
    click_at(&mut ctx, x_after_hello, content.y + content.height / 2.0);
    assert_eq!(doc(&ctx).control_edit_state(node).map(|s| s.caret), Some(5));
    type_text(&mut ctx, "X");
    assert_eq!(value(&ctx, "t"), "helloX world");

    // Far right of the text → caret at the end; far left → at the start.
    click_at(
        &mut ctx,
        content.x + content.width - 1.0,
        content.y + content.height / 2.0,
    );
    type_text(&mut ctx, "!");
    assert_eq!(value(&ctx, "t"), "helloX world!");
    click_at(&mut ctx, content.x + 1.0, content.y + content.height / 2.0);
    type_text(&mut ctx, ">");
    assert_eq!(value(&ctx, "t"), ">helloX world!");
}

#[test]
fn click_places_the_caret_on_a_textarea_line() {
    let mut ctx = page(
        r#"<textarea id="ta" rows="4" cols="20">ab
cd
ef</textarea>"#,
    );
    let node = by_id(&ctx, "ta");
    let (content, font) = text_box(&ctx, node);
    let line_h = font.line_height.max(font.size);
    // Start of the second line.
    click_at(&mut ctx, content.x + 1.0, content.y + line_h * 1.5);
    type_text(&mut ctx, "Z");
    assert_eq!(value(&ctx, "ta"), "ab\nZcd\nef");
    // End of the third line.
    click_at(&mut ctx, content.x + content.width - 1.0, content.y + line_h * 2.5);
    type_text(&mut ctx, "!");
    assert_eq!(value(&ctx, "ta"), "ab\nZcd\nef!");
}

#[test]
fn click_places_the_caret_on_a_soft_wrapped_row() {
    let mut ctx = page(
        r#"<textarea id="ta" rows="5" cols="20">The quick brown fox jumps over the lazy dog and keeps on running</textarea>"#,
    );
    let node = by_id(&ctx, "ta");
    let (content, font) = text_box(&ctx, node);
    let line_h = font.line_height.max(font.size);
    let rows = {
        let fs = ctx.font_system();
        let mut fs = fs.lock();
        let width = content.width - 2.0 * text_field::inset_x(content.width);
        text_field::layout_rows(
            &mut *fs,
            "The quick brown fox jumps over the lazy dog and keeps on running",
            &font,
            width,
        )
    };
    assert!(rows.len() >= 3, "the text must wrap into several rows: {rows:?}");
    // Click at the very start of the second visual row: the caret goes to that row's first char.
    click_at(&mut ctx, content.x + 1.0, content.y + line_h * 1.5);
    assert_eq!(doc(&ctx).control_edit_state(node).map(|s| s.caret), Some(rows[1].start));
    type_text(&mut ctx, "|");
    let v = value(&ctx, "ta");
    assert_eq!(&v[rows[1].start..=rows[1].start], "|");
}

// ── selection / clipboard / textarea navigation ───────────────────────────────

#[test]
fn shift_arrows_select_and_typing_replaces() {
    let mut ctx = page(r#"<input id="t" value="hello world">"#);
    click(&mut ctx, "t");
    key(&mut ctx, "End");
    chord(&mut ctx, "ArrowLeft", true, true); // Ctrl+Shift+Left: select "world"
    assert_eq!(selection(&ctx, "t"), Some((6, 11)));
    type_text(&mut ctx, "there");
    assert_eq!(value(&ctx, "t"), "hello there");
    assert_eq!(selection(&ctx, "t"), None);
    chord(&mut ctx, "a", true, false);
    assert_eq!(selection(&ctx, "t"), Some((0, 11)));
    key(&mut ctx, "Backspace");
    assert_eq!(value(&ctx, "t"), "");
    // Plain arrows collapse a selection to its edge instead of moving by one.
    type_text(&mut ctx, "abc");
    chord(&mut ctx, "Home", false, true);
    assert_eq!(selection(&ctx, "t"), Some((0, 3)));
    key(&mut ctx, "ArrowRight");
    assert_eq!((caret(&ctx, "t"), selection(&ctx, "t")), (Some(3), None));
}

#[test]
fn clipboard_goes_through_the_embedder() {
    let mut ctx = page(r#"<input id="t" value="copy me"><input id="p" type="password" value="secret">"#);
    click(&mut ctx, "t");
    chord(&mut ctx, "a", true, false);
    chord(&mut ctx, "c", true, false);
    assert_eq!(ctx.take_clipboard_write().as_deref(), Some("copy me"));
    assert_eq!(ctx.take_clipboard_write(), None);
    chord(&mut ctx, "x", true, false);
    assert_eq!(ctx.take_clipboard_write().as_deref(), Some("copy me"));
    assert_eq!(value(&ctx, "t"), "");
    chord(&mut ctx, "v", true, false);
    assert!(ctx.take_paste_request());
    assert!(!ctx.take_paste_request());
    // The embedder answers with the clipboard text; line breaks don't survive a single-line field.
    ctx.insert_text("pasted\nline");
    assert_eq!(value(&ctx, "t"), "pastedline");
    // Password fields never hand out their text.
    click(&mut ctx, "p");
    chord(&mut ctx, "a", true, false);
    chord(&mut ctx, "c", true, false);
    assert_eq!(ctx.take_clipboard_write(), None);
}

#[test]
fn tab_into_a_text_field_selects_its_text() {
    let mut ctx = page(r#"<input id="a" value="first"><input id="b" value="second"><textarea id="c">body</textarea>"#);
    key(&mut ctx, "Tab");
    assert_eq!(selection(&ctx, "a"), Some((0, 5)));
    key(&mut ctx, "Tab");
    assert_eq!(selection(&ctx, "b"), Some((0, 6)));
    type_text(&mut ctx, "x");
    assert_eq!(value(&ctx, "b"), "x");
    // Textareas keep their caret (at the top) instead.
    key(&mut ctx, "Tab");
    assert_eq!(selection(&ctx, "c"), None);
    type_text(&mut ctx, ">");
    assert_eq!(value(&ctx, "c"), ">body");
}

#[test]
fn mouse_drag_and_double_click_select() {
    let mut ctx = page(r#"<input id="t" size="30" value="alpha beta gamma">"#);
    let node = by_id(&ctx, "t");
    let (content, font) = text_box(&ctx, node);
    let x_of = |ctx: &Ctx, chars: usize| {
        let fs = ctx.font_system();
        let mut fs = fs.lock();
        let prefix: String = "alpha beta gamma".chars().take(chars).collect();
        content.x + text_field::inset_x(content.width) + text_field::width(&mut *fs, &prefix, &font)
    };
    let y = content.y + content.height / 2.0;
    // Press at "beta", drag to the end of "gamma".
    let (x0, x1) = (x_of(&ctx, 6), x_of(&ctx, 16));
    ctx.focus_at(x0, y);
    ctx.activate_at(x0, y);
    ctx.drag_move(x1 + 2.0, y);
    ctx.end_drag();
    assert_eq!(selection(&ctx, "t"), Some((6, 16)));
    // Two quick presses on "alpha" select the word.
    let xa = x_of(&ctx, 2);
    for _ in 0..2 {
        ctx.focus_at(xa, y);
        ctx.activate_at(xa, y);
        ctx.end_drag();
    }
    assert_eq!(selection(&ctx, "t"), Some((0, 5)));
}

#[test]
fn textarea_row_keys_and_scrolling() {
    let lines: Vec<String> = (1..=12).map(|i| format!("line {i}")).collect();
    let text = lines.join("\n");
    let mut ctx = page(&format!(r#"<textarea id="ta" rows="4" cols="20">{text}</textarea>"#));
    let node = by_id(&ctx, "ta");
    let (content, font) = text_box(&ctx, node);
    click_at(&mut ctx, content.x + 1.0, content.y + 1.0);
    assert_eq!(caret(&ctx, "ta"), Some(0));
    key(&mut ctx, "ArrowDown");
    key(&mut ctx, "ArrowDown");
    assert_eq!(caret(&ctx, "ta"), Some(text.find("line 3").unwrap_or(0)));
    key(&mut ctx, "End");
    assert_eq!(caret(&ctx, "ta"), Some(text.find("line 3").unwrap_or(0) + 6));
    chord(&mut ctx, "ArrowUp", false, true);
    assert_eq!(
        selection(&ctx, "ta"),
        Some((
            text.find("line 2").unwrap_or(0) + 6,
            text.find("line 3").unwrap_or(0) + 6
        ))
    );
    // Moving below the four visible rows scrolls the view to keep the caret inside.
    for _ in 0..5 {
        key(&mut ctx, "ArrowDown");
    }
    let scroll = doc(&ctx).control_edit_state(node).map(|s| s.scroll);
    assert_eq!(scroll, Some(3), "the caret is on row 6 (0-based); 4 rows fit");
    // The wheel scrolls independently of the caret; typing brings the caret back into view.
    let (cx, cy) = (content.x + 5.0, content.y + 5.0);
    ctx.update_hover(cx, cy);
    assert!(ctx.area_scroll(cx, cy, -200.0));
    assert_eq!(doc(&ctx).control_edit_state(node).map(|s| s.scroll), Some(0));
    type_text(&mut ctx, "!");
    assert_eq!(doc(&ctx).control_edit_state(node).map(|s| s.scroll), Some(3));
    // The scrollbar is there: the text block is narrower than the content box.
    let area = {
        let fs = ctx.font_system();
        let mut fs = fs.lock();
        text_field::area_layout(&mut *fs, &value(&ctx, "ta"), &font, content)
    };
    assert!(area.track.is_some());
    assert!(area.text.width < content.width - 8.0);
}

#[test]
fn shift_up_across_a_soft_wrap_keeps_going() {
    let text = "line 1 the first\nline 2\nline 3\nline 4\nline 5 has a few more words so it wraps around\nline 6\nline 7\nline 8\nline 9\nline 10\nline 11\nline 12 the last";
    let mut ctx = page(&format!(r#"<textarea id="ta" rows="4" cols="40">{text}</textarea>"#));
    let node = by_id(&ctx, "ta");
    let (content, font) = text_box(&ctx, node);
    let rows = {
        let fs = ctx.font_system();
        let mut fs = fs.lock();
        text_field::area_layout(&mut *fs, text, &font, content).rows
    };
    // Line 5 wraps once: rows 4 (soft) and 5 (hard), then "line 6" is row 6.
    assert!(!rows[4].hard_end && rows[5].hard_end && rows.len() == 13, "{rows:?}");
    let (cx, cy) = (content.x + 5.0, content.y + 5.0);
    ctx.update_hover(cx, cy);
    assert!(ctx.area_scroll(cx, cy, 200.0));
    assert_eq!(doc(&ctx).control_edit_state(node).map(|s| s.scroll), Some(5));
    // Third visible row is row 7 ("line 7"); click past its end.
    click_at(&mut ctx, content.x + 100.0, content.y + 2.5 * 18.0);
    assert_eq!(caret(&ctx, "ta"), Some(rows[7].end));
    key(&mut ctx, "ArrowUp");
    assert_eq!(caret(&ctx, "ta"), Some(rows[6].end));
    // Up onto the wrapped tail keeps the column (6 chars in on both rows); the anchor stays.
    chord(&mut ctx, "ArrowUp", false, true);
    assert_eq!(
        caret(&ctx, "ta"),
        Some(rows[5].start + 6.min(rows[5].end - rows[5].start))
    );
    chord(&mut ctx, "ArrowUp", false, true);
    assert_eq!(caret(&ctx, "ta"), Some(rows[4].start + 6));
    assert_eq!(doc(&ctx).control_edit_state(node).map(|s| s.scroll), Some(4));
    chord(&mut ctx, "End", false, true);
    // End of a soft-wrapped row is the wrap point.
    assert_eq!(caret(&ctx, "ta"), Some(rows[4].end));
    assert_eq!(selection(&ctx, "ta"), Some((rows[4].end, rows[6].end)));
}

#[test]
fn caret_after_a_space_sits_past_the_full_space() {
    let ctx = page(r#"<input id="t" size="30" value="hello   world">"#);
    let node = by_id(&ctx, "t");
    let (_, font) = text_box(&ctx, node);
    let fs = ctx.font_system();
    let mut fs = fs.lock();
    let space = text_field::space_advance(&mut *fs, &font);
    assert!(space > font.size * 0.15, "a real space advance, got {space}");
    // Caret x after "hello " = width("hello") + one real space, not a 0.3em guess.
    let w5 = text_field::width(&mut *fs, "hello", &font);
    let x6 = text_field::x_in_row(&mut *fs, "hello   world", 6, &font);
    assert!((x6 - (w5 + space)).abs() < 0.5, "x6={x6} w5={w5} space={space}");
    // Each boundary inside the space run gets its own x, and clicks map back to it.
    let x7 = text_field::x_in_row(&mut *fs, "hello   world", 7, &font);
    assert!((x7 - x6 - space).abs() < 0.5);
    assert_eq!(text_field::index_at_x(&mut *fs, "hello   world", &font, x7 + 1.0), 7);
}

// ── cursor shape ──────────────────────────────────────────────────────────────

#[test]
fn cursor_kind_follows_whats_under_the_pointer() {
    use crate::engine::events::CursorShape as CursorKind;
    let lines: Vec<String> = (1..=9).map(|i| format!("row {i}")).collect();
    let mut ctx = page(&format!(
        r#"<input id="t" value="x"> <button id="b">Go</button> <a id="l" href="/x">link</a>
           <textarea id="ta" rows="3" cols="20">{}</textarea>
           <input id="d" disabled value="y">"#,
        lines.join("\n")
    ));
    let probe = |ctx: &mut Ctx, id: &str| {
        let (x, y) = center(ctx, by_id(ctx, id));
        ctx.update_hover(x, y);
        ctx.cursor_at(x, y)
    };
    assert_eq!(probe(&mut ctx, "t"), CursorKind::Text);
    assert_eq!(probe(&mut ctx, "b"), CursorKind::Default);
    assert_eq!(probe(&mut ctx, "l"), CursorKind::Pointer);
    assert_eq!(probe(&mut ctx, "ta"), CursorKind::Text);
    // A disabled field doesn't invite typing.
    assert_eq!(probe(&mut ctx, "d"), CursorKind::Default);
    // The textarea's grip corner resizes; its scrollbar strip is not text.
    let ta = by_id(&ctx, "ta");
    let bb = border_box(&ctx, ta);
    let (gx, gy) = (bb.x + bb.width - 4.0, bb.y + bb.height - 4.0);
    ctx.update_hover(gx, gy);
    assert_eq!(ctx.cursor_at(gx, gy), CursorKind::Resize);
    let (content, _) = text_box(&ctx, ta);
    let (sx, sy) = (content.x + content.width - 3.0, content.y + 3.0);
    ctx.update_hover(sx, sy);
    assert_eq!(ctx.cursor_at(sx, sy), CursorKind::Default);
}
