//! CSS counters: scoping, the implicit `list-item` counter, and `counter()` / `counters()` in
//! generated content.

use std::sync::Arc;

use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_shared::node::NodeId;

use crate::common::document::node::NodeType;
use crate::common::document::pipeline_doc::{is_marker_box, GosubDocumentAdapter, PipelineDocument};

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

fn adapter(html: &str) -> GosubDocumentAdapter<Config> {
    let mut doc = html_compile::<Config>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    GosubDocumentAdapter::<Config>::new(Arc::new(doc))
}

fn element(a: &GosubDocumentAdapter<Config>, id: &str) -> NodeId {
    fn find(a: &GosubDocumentAdapter<Config>, node: NodeId, id: &str) -> Option<NodeId> {
        if a.doc.attribute(node, "id") == Some(id) {
            return Some(node);
        }
        a.doc.children(node).iter().find_map(|&child| find(a, child, id))
    }
    find(a, a.doc.root(), id).unwrap_or_else(|| panic!("no element with id {id}"))
}

/// All the text in a box subtree, generated text included, in box order.
fn text_below(a: &GosubDocumentAdapter<Config>, node: NodeId, out: &mut String) {
    for child in a.children(node) {
        match a.get_node_by_id(child).map(|node| node.node_type) {
            Some(NodeType::Text(text)) => out.push_str(&text),
            _ => text_below(a, child, out),
        }
    }
}

/// The text the element with this `id` renders, its `::before` and `::after` included.
fn text(a: &GosubDocumentAdapter<Config>, id: &str) -> String {
    let mut out = String::new();
    text_below(a, element(a, id), &mut out);
    out
}

/// The text of the marker of the element with this `id`; `None` when it has no marker.
fn marker(a: &GosubDocumentAdapter<Config>, id: &str) -> Option<String> {
    let marker = a
        .children(element(a, id))
        .into_iter()
        .find(|&child| is_marker_box(child))?;
    let mut out = String::new();
    text_below(a, marker, &mut out);
    Some(out)
}

#[test]
fn list_items_count_through_the_elements_around_them() {
    let a = adapter(
        r#"<ol>
            <li id="a">a</li>
            <div><li id="b">b</li><li id="c">c</li></div>
            <section><div><li id="d">d</li></div></section>
            <li id="e">e</li>
        </ol>"#,
    );
    let markers: Vec<_> = ["a", "b", "c", "d", "e"]
        .map(|id| marker(&a, id).unwrap_or_default())
        .into();
    assert_eq!(markers, ["1. ", "2. ", "3. ", "4. ", "5. "]);
}

#[test]
fn a_nested_list_starts_its_own_count() {
    let a = adapter(r#"<ol><li id="a">a<ol><li id="a1">x</li><li id="a2">y</li></ol></li><li id="b">b</li></ol>"#);
    assert_eq!(marker(&a, "a1").as_deref(), Some("1. "));
    assert_eq!(marker(&a, "a2").as_deref(), Some("2. "));
    // The nested list's counter ends with it; the outer one carries on.
    assert_eq!(marker(&a, "b").as_deref(), Some("2. "));
}

#[test]
fn authors_can_reset_and_step_the_list_item_counter() {
    let a = adapter(
        r#"<style>
            #five { counter-reset: list-item 5 }
            #evens > li { counter-increment: list-item 2 }
            #frozen { counter-increment: list-item 0 }
        </style>
        <ol id="five"><li id="f1">a</li><li id="f2">b</li></ol>
        <ol id="evens"><li id="e1">a</li><li id="e2">b</li><li id="e3">c</li></ol>
        <ol><li id="z1">a</li><li id="frozen">b</li><li id="z3">c</li></ol>"#,
    );
    assert_eq!(marker(&a, "f1").as_deref(), Some("6. "));
    assert_eq!(marker(&a, "f2").as_deref(), Some("7. "));
    // Naming `list-item` replaces the implicit increment rather than adding to it (§4.6).
    assert_eq!(marker(&a, "e1").as_deref(), Some("2. "));
    assert_eq!(marker(&a, "e3").as_deref(), Some("6. "));
    assert_eq!(marker(&a, "z1").as_deref(), Some("1. "));
    assert_eq!(marker(&a, "frozen").as_deref(), Some("1. "));
    assert_eq!(marker(&a, "z3").as_deref(), Some("2. "));
}

#[test]
fn a_summary_does_not_take_a_number() {
    let a = adapter(
        r#"<style>details > summary { list-style-type: decimal }</style>
        <ol><li id="a">a</li><li><details><summary id="s">s</summary></details></li><li id="c">c</li></ol>"#,
    );
    // The UA's `counter-increment: list-item 0`: the summary shows the item's number, and
    // does not move it on.
    assert_eq!(marker(&a, "s").as_deref(), Some("2. "));
    assert_eq!(marker(&a, "c").as_deref(), Some("3. "));
}

#[test]
fn reversed_lists_count_down_to_one() {
    let a = adapter(
        r#"<ol reversed><li id="r1">a</li><li id="r2" value="10">b</li><li id="r3">c</li></ol>
        <ol reversed><li id="w1">a</li><div><li id="w2">b</li></div><li id="w3">c</li></ol>"#,
    );
    // §4.4.2's start value: the set item's 10, plus the one decrement before it, plus one.
    assert_eq!(marker(&a, "r1").as_deref(), Some("11. "));
    assert_eq!(marker(&a, "r2").as_deref(), Some("10. "));
    assert_eq!(marker(&a, "r3").as_deref(), Some("9. "));
    // An item inside a wrapper still counts toward the start.
    assert_eq!(marker(&a, "w1").as_deref(), Some("3. "));
    assert_eq!(marker(&a, "w3").as_deref(), Some("1. "));
}

#[test]
fn counters_joins_every_level() {
    // The tiered example of css-lists-3 §4.6.
    let a = adapter(
        r#"<style>ol > li::marker { content: counters(list-item, '.') '. ' }</style>
        <ol>
          <li id="one">a
          <li id="two" value=5>b
            <ol start=3>
              <li id="three">c
              <li id="four">d
                <ol reversed>
                  <li id="r4">e
                  <li>f
                  <li>g
                  <li id="r1">h
                </ol>
              <li id="five">i
            </ol>
          <li id="six">j
        </ol>"#,
    );
    assert_eq!(marker(&a, "one").as_deref(), Some("1. "));
    assert_eq!(marker(&a, "two").as_deref(), Some("5. "));
    assert_eq!(marker(&a, "three").as_deref(), Some("5.3. "));
    assert_eq!(marker(&a, "four").as_deref(), Some("5.4. "));
    assert_eq!(marker(&a, "r4").as_deref(), Some("5.4.4. "));
    assert_eq!(marker(&a, "r1").as_deref(), Some("5.4.1. "));
    assert_eq!(marker(&a, "five").as_deref(), Some("5.5. "));
    assert_eq!(marker(&a, "six").as_deref(), Some("6. "));
}

#[test]
fn counter_writes_in_the_style_it_names() {
    let a = adapter(
        r#"<style>
            body { counter-reset: n 3 }
            #roman::before { content: counter(n, upper-roman) ". " }
            #alpha::before { content: "(" counter(n, lower-alpha) ")" }
            #plain::before { content: counter(n) }
            #none::before { content: "[" counter(n, none) "]" }
            #disc::before { content: counter(n, disc) }
        </style>
        <p id="roman"></p><p id="alpha"></p><p id="plain"></p><p id="none"></p><p id="disc"></p>"#,
    );
    assert_eq!(text(&a, "roman"), "III. ");
    assert_eq!(text(&a, "alpha"), "(c)");
    assert_eq!(text(&a, "plain"), "3");
    assert_eq!(text(&a, "none"), "[]");
    // A symbolic style without the marker's trailing space.
    assert_eq!(text(&a, "disc"), "\u{2022}");
}

#[test]
fn chapters_and_sections_number_from_before() {
    // The example of css-lists-3 §4.2: the pseudo-elements do the counting. The example has
    // no `counter-reset`; without one each `::before` would create its own counter, which only
    // it is in the scope of (see the next test), so the body makes them.
    let a = adapter(
        r#"<style>
            body { counter-reset: chapter section }
            h1::before { content: "Chapter " counter(chapter) ". "; counter-increment: chapter; counter-set: section }
            h2::before { content: counter(chapter) "." counter(section) " "; counter-increment: section }
        </style>
        <h1 id="c1">A</h1><h2 id="s11">a</h2><h2 id="s12">b</h2>
        <h1 id="c2">B</h1><h2 id="s21">c</h2>"#,
    );
    assert_eq!(text(&a, "c1"), "Chapter 1. A");
    assert_eq!(text(&a, "s11"), "1.1 a");
    assert_eq!(text(&a, "s12"), "1.2 b");
    assert_eq!(text(&a, "c2"), "Chapter 2. B");
    assert_eq!(text(&a, "s21"), "2.1 c");
}

#[test]
fn a_value_comes_from_the_element_before_in_tree_order() {
    // The example of css-lists-3 §4.4.1: #baz inherits the counter from its sibling #foo, and
    // the value 2 from #bar inside it.
    let a = adapter(
        r#"<style>#baz::before, #bar::after { content: counter(example) }</style>
        <ul style="counter-reset: example 0">
          <li id="foo" style="counter-increment: example">foo
            <div id="bar" style="counter-increment: example">bar</div>
          </li>
          <li id="baz">baz</li>
        </ul>"#,
    );
    assert_eq!(text(&a, "bar"), "bar2");
    assert_eq!(text(&a, "baz"), "\u{2022} 2baz");
}

#[test]
fn a_counter_made_by_a_pseudo_element_stays_its_own() {
    // WPT css-lists/counters-scope-003 and -004: what a pseudo-element creates nothing else is
    // in the scope of, but what it does to a counter it inherited carries on in tree order.
    let a = adapter(
        r#"<style>
            #s003 span::before { counter-increment: c 1; content: "B" counters(c, ".") "-" }
            #s003 span::after { counter-increment: c 1; content: "A" counters(c, ".") "-" }
            #s003, #s003 #reset::before { counter-reset: c 0 }
            .reset { counter-reset: d }
            .rb::before { counter-reset: d; content: "R" }
            .use { counter-increment: d }
            .use::before { content: counters(d, ".") " " }
        </style>
        <div id="s003"><span id="outer"><span id="reset"><span></span></span></span></div>
        <div>
          <span class="reset"></span><span class="use" id="u1"></span>
          <span class="reset"></span><span class="use" id="u2"></span>
          <span class="rb" id="rb"><span class="use" id="u3"></span></span>
        </div>"#,
    );
    assert_eq!(text(&a, "outer"), "B1-B1.1-B2-A3-A4-A5-");
    assert_eq!(text(&a, "u1"), "1 ");
    assert_eq!(text(&a, "u2"), "1 ");
    // The reset on `.rb::before` is not seen inside `.rb`.
    assert_eq!(text(&a, "rb"), "R2 ");
}

#[test]
fn a_reset_on_a_later_sibling_replaces_the_counter() {
    let a = adapter(
        r#"<style>
            .r { counter-reset: c 10 }
            .i { counter-increment: c }
            .show::before { content: counters(c, "-") }
        </style>
        <div class="r"></div><div class="i show" id="a"></div>
        <div class="r"></div><div class="i show" id="b"></div>
        <div class="r"><span class="i show" id="nested"></span><div class="r"><span class="i show" id="inner"></span></div></div>"#,
    );
    assert_eq!(text(&a, "a"), "11");
    // The second reset does not nest inside the first: it was made by a sibling.
    assert_eq!(text(&a, "b"), "11");
    assert_eq!(text(&a, "nested"), "11");
    // Inside an element that resets it, a counter of the same name nests.
    assert_eq!(text(&a, "inner"), "11-11");
}

#[test]
fn reset_increment_and_set_apply_in_that_order() {
    let a = adapter(
        r#"<style>
            #one { counter-reset: x 1; counter-increment: x 5; counter-set: x 2 }
            #two { counter-increment: x 5 }
            #twice { counter-increment: y 2 y 3; counter-reset: z 1 z 7 }
            #one::after, #two::after { content: counter(x) }
            #twice::after { content: counter(y) "/" counter(z) }
        </style>
        <div id="one"></div><div id="two"></div><div id="twice"></div>"#,
    );
    assert_eq!(text(&a, "one"), "2");
    assert_eq!(text(&a, "two"), "7");
    // Increments of one name add up; of several resets of one name only the last counts.
    assert_eq!(text(&a, "twice"), "5/7");
}

#[test]
fn a_box_that_is_not_generated_does_not_count() {
    // css-lists-3 §4.5, and WPT counter-reset-increment-set-display-contents (which wants 7).
    let a = adapter(
        r#"<style>
            .inc { counter-increment: x }
            .reset-6 { counter-reset: x 6 }
            .reset-666 { counter-reset: x 666 }
            .set-666 { counter-set: x 666 }
            .contents { display: contents }
            .gone { display: none }
            .result::before { content: counter(x) }
            .nobefore::before { content: none; counter-increment: x 100 }
            .hidden { visibility: hidden }
        </style>
        <div>
          <span class="reset-6"></span>
          <span class="contents reset-666 inc"></span>
          <span class="contents set-666"></span>
          <span class="gone reset-666 inc"><span class="inc"></span></span>
          <span class="nobefore"></span>
          <span class="inc result" id="seven"></span>
          <span class="hidden inc"></span>
          <span class="result" id="eight"></span>
        </div>"#,
    );
    assert_eq!(text(&a, "seven"), "7");
    // A hidden box still generates a box, and still counts.
    assert_eq!(text(&a, "eight"), "8");
}

#[test]
fn a_display_contents_element_still_lets_its_children_count() {
    let a = adapter(
        r#"<style>
            .contents { display: contents }
            .inc { counter-increment: x }
            #out::before { content: counter(x) }
        </style>
        <div style="counter-reset: x"><div class="contents"><span class="inc"></span><span class="inc"></span></div><span id="out"></span></div>"#,
    );
    assert_eq!(text(&a, "out"), "2");
}

#[test]
fn counters_reach_across_display_contents_wrappers() {
    // WPT css-lists/counter-order-display-contents: the wrappers have no box, so the counter
    // the first item makes is its siblings' as well, as it is in every engine.
    let a = adapter(
        r#"<style>
            .counted { counter-increment: section }
            .counted::before { content: "C=" counter(section) " " }
        </style>
        <div style="display: contents"><div class="counted" id="one">One</div></div>
        <div style="display: contents"><div class="counted" id="two">Two</div></div>
        <div style="display: contents"><div class="counted" id="three">Three</div></div>"#,
    );
    assert_eq!(text(&a, "one"), "C=1 One");
    assert_eq!(text(&a, "two"), "C=2 Two");
    assert_eq!(text(&a, "three"), "C=3 Three");
}

#[test]
fn a_display_contents_element_keeps_its_own_pseudo_elements_counting() {
    // Its `::before` reads the counters while the walk is still being asked for its children:
    // that has to be answered without building the pseudo-box again.
    let a = adapter(
        r#"<style>
            .contents { display: contents; counter-increment: x 50 }
            .contents::before { content: "[" counter(x) "]"; counter-increment: x }
            .contents::after { content: "(" counter(x) ")" }
            .inc { counter-increment: x }
        </style>
        <div id="wrap" style="counter-reset: x"><div class="contents"><span class="inc"></span></div></div>"#,
    );
    // The element itself does not count; its `::before` and its child do.
    assert_eq!(text(&a, "wrap"), "[1](2)");
}

#[test]
fn slotted_elements_count_in_flat_tree_order() {
    let a = adapter(
        r#"<style>.inc { counter-increment: x } .inc::before { content: counter(x) }</style>
        <div id="host" style="counter-reset: x">
          <template shadowrootmode="open"><style>.inc { counter-increment: x } .inc::before { content: counter(x) }</style><span class="inc"></span><slot></slot><span class="inc"></span></template>
          <span class="inc"></span>
        </div>"#,
    );
    // Document styles stop at the shadow root, so the shadow tree brings its own. Only the end
    // of the text is compared: the shadow tree's `<style>` text is in the box tree as well.
    let text = text(&a, "host").split_whitespace().collect::<String>();
    assert!(text.ends_with("}123"), "{text}");
}

#[test]
fn a_reversed_start_is_clamped_too() {
    let a = adapter(
        r#"<style>
            #r { counter-reset: reversed(x) }
            #r > span { counter-increment: x -2147483647 }
            #r > span::before { content: counter(x) " " }
        </style>
        <div id="r"><span></span><span></span></div>"#,
    );
    // The start works out at 3 * 2147483647, clamped to 2147483647 before it is counted from.
    assert_eq!(text(&a, "r"), "0 -2147483647 ");
}

#[test]
fn a_counter_nobody_made_reads_zero() {
    let a =
        adapter(r#"<style>#a::before { content: counter(nothing) "|" counters(nothing, ".") }</style><p id="a"></p>"#);
    assert_eq!(text(&a, "a"), "0|0");
}

#[test]
fn counters_are_clamped_to_32_bits() {
    // WPT counter-reset-increment-overflow-underflow.
    let a = adapter(
        r#"<style>
            .over { counter-reset: o 2000000000 }
            .over h3::before { counter-increment: o 50000000; content: counter(o) }
            .under { counter-reset: u -2000000000 }
            .under h3::before { counter-increment: u -50000000; content: counter(u) }
            #exact::before { content: counter(e) }
        </style>
        <div class="over"><h3></h3><h3></h3><h3 id="o3"></h3></div>
        <div class="under"><h3></h3><h3></h3><h3 id="u3"></h3></div>
        <p id="exact" style="counter-reset: e 16777217"></p>"#,
    );
    assert_eq!(text(&a, "o3"), "2147483647");
    assert_eq!(text(&a, "u3"), "-2147483648");
    // Not rounded through an f32 on the way.
    assert_eq!(text(&a, "exact"), "16777217");
}

#[test]
fn function_names_are_case_insensitive_and_counter_names_are_not() {
    let a = adapter(
        r#"<style>
            body { counter-reset: Chapter 2 chapter 7 }
            #up::before { content: COUNTER(Chapter) "|" Counters(chapter, ".") }
            #rev { counter-reset: REVERSED(n) }
            #rev > span { counter-increment: n -1 }
            #rev > span::before { content: counter(n) }
        </style>
        <p id="up"></p><div id="rev"><span></span><span></span></div>"#,
    );
    assert_eq!(text(&a, "up"), "2|7");
    assert_eq!(text(&a, "rev"), "21");
}

#[test]
fn bullets_do_not_resolve_the_counters() {
    let a = adapter(r#"<ul><li id="a">a</li><li id="b">b</li></ul>"#);
    assert_eq!(marker(&a, "b").as_deref(), Some("\u{2022} "));
    assert!(!a.counters_resolved());
    let a = adapter(r#"<ol><li id="a">a</li><li id="b">b</li></ol>"#);
    assert_eq!(marker(&a, "b").as_deref(), Some("2. "));
    assert!(a.counters_resolved());
}

#[test]
fn an_element_that_never_renders_does_not_count() {
    // `head` gets no computed style at all, so no `display: none` either; the render tree drops
    // it by name, and the counters have to as well.
    let a = adapter(
        r#"<html style="counter-reset: x"><head><style>
            head::before, title::before { content: ""; counter-increment: x }
            #out::before { content: counter(x) }
        </style><title>t</title></head><body><p id="out"></p></body></html>"#,
    );
    assert_eq!(text(&a, "out"), "0");
}
