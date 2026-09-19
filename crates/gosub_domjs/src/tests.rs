//! Smoke tests: the bindings must read the real document, not a JS-side copy.

use rquickjs::{CatchResultExt, Context, Runtime};

use crate::timers::{TimerState, Timers};
use crate::{install, parse_document};

/// Run `script`, drain the timer queue, then read `after` back out - the timer tests need to
/// observe state that only exists once the queue has been pumped.
fn eval_after_timers(html: &str, script: &str, after: &str) -> String {
    let (doc, _) = parse_document(html, None).expect("parse");
    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let timers: Timers = std::rc::Rc::new(std::cell::RefCell::new(TimerState::default()));
    context.with(|ctx| {
        install(&ctx, doc, &timers).expect("install");
        ctx.eval::<(), _>(script).catch(&ctx).expect("eval");
        while crate::timers::run_next(&ctx, &timers).expect("timers") {}
        ctx.eval::<String, _>(after).catch(&ctx).expect("read back")
    })
}

fn eval(html: &str, script: &str) -> String {
    let (doc, _) = parse_document(html, None).expect("parse");
    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let timers: Timers = std::rc::Rc::new(std::cell::RefCell::new(TimerState::default()));
    context.with(|ctx| {
        install(&ctx, doc, &timers).expect("install");
        let result = ctx.eval::<String, _>(script).catch(&ctx).expect("eval");
        while crate::timers::run_next(&ctx, &timers).expect("timers") {}
        result
    })
}

#[test]
fn reads_parsed_markup() {
    let value = eval(
        "<select><option value=a>First</option><option>Second</option></select>",
        "document.getElementsByTagName('option')[1].value",
    );
    assert_eq!(value, "Second");
}

#[test]
fn option_value_falls_back_to_stripped_and_collapsed_text() {
    let value = eval(
        "<option> child  node </option>",
        "document.querySelector('option').value",
    );
    assert_eq!(value, "child node");
}

#[test]
fn mutations_land_in_the_document() {
    let html = eval(
        "<body><p id=target></p></body>",
        "const p = document.getElementById('target');
         p.appendChild(document.createTextNode('hi'));
         p.setAttribute('data-x', '1');
         p.outerHTML",
    );
    assert!(html.contains("hi"), "{html}");
    assert!(html.contains("data-x"), "{html}");
}

#[test]
fn node_wrappers_have_stable_identity() {
    let same = eval(
        "<div id=parent><span id=child></span></div>",
        "String(document.getElementById('child').parentNode === document.getElementById('parent'))",
    );
    assert_eq!(same, "true");
}

#[test]
fn listeners_run_in_capture_target_bubble_order() {
    let order = eval(
        "<div id=outer><span id=inner></span></div>",
        "const seen = [];
         const outer = document.getElementById('outer');
         const inner = document.getElementById('inner');
         outer.addEventListener('x', () => seen.push('capture'), true);
         outer.addEventListener('x', () => seen.push('bubble'));
         inner.addEventListener('x', e => seen.push('target' + e.eventPhase));
         inner.dispatchEvent(new Event('x', {bubbles: true}));
         seen.join(',')",
    );
    assert_eq!(order, "capture,target2,bubble");
}

#[test]
fn duplicate_listeners_are_ignored_and_once_runs_once() {
    let count = eval(
        "<span id=el></span>",
        "let n = 0;
         const el = document.getElementById('el');
         const handler = () => n++;
         el.addEventListener('y', handler, {once: true});
         el.addEventListener('y', handler, {once: true});
         el.dispatchEvent(new Event('y'));
         el.dispatchEvent(new Event('y'));
         String(n)",
    );
    assert_eq!(count, "1");
}

#[test]
fn prevent_default_is_reported_by_dispatch_event() {
    let result = eval(
        "<span id=el></span>",
        "const el = document.getElementById('el');
         el.addEventListener('z', e => e.preventDefault());
         String(el.dispatchEvent(new Event('z', {cancelable: true})))",
    );
    assert_eq!(result, "false");
}

#[test]
fn stop_propagation_keeps_an_event_from_the_parent() {
    let seen = eval(
        "<div id=outer><span id=inner></span></div>",
        "const seen = [];
         document.getElementById('outer').addEventListener('s', () => seen.push('outer'));
         const inner = document.getElementById('inner');
         inner.addEventListener('s', e => { seen.push('inner'); e.stopPropagation(); });
         inner.dispatchEvent(new Event('s', {bubbles: true}));
         seen.join(',')",
    );
    assert_eq!(seen, "inner");
}

#[test]
fn timers_fire_in_due_order_not_registration_order() {
    let order = eval_after_timers(
        "<span></span>",
        "globalThis.seen = [];
         setTimeout(() => seen.push('late'), 50);
         setTimeout(() => seen.push('early'), 1);
         setTimeout(() => seen.push('same-time-second'), 1);",
        "seen.join(',')",
    );
    assert_eq!(order, "early,same-time-second,late");
}

#[test]
fn a_cleared_timer_never_runs() {
    let ran = eval_after_timers(
        "<span></span>",
        "globalThis.ran = false;
         const id = setTimeout(() => { ran = true; }, 5);
         clearTimeout(id);",
        "String(ran)",
    );
    assert_eq!(ran, "false");
}

#[test]
fn request_animation_frame_delivers_a_timestamp() {
    let kind = eval_after_timers(
        "<span></span>",
        "globalThis.kind = 'never ran';
         requestAnimationFrame(ts => { kind = typeof ts; });",
        "kind",
    );
    assert_eq!(kind, "number");
}

#[test]
fn click_reaches_a_listener_on_an_ancestor() {
    let seen = eval(
        "<form id=f><button id=b>go</button></form>",
        "const seen = [];
         document.getElementById('f').addEventListener('click', e => seen.push(e.target.tagName));
         document.getElementById('b').click();
         seen.join(',')",
    );
    assert_eq!(seen, "BUTTON");
}

#[test]
fn get_elements_by_tag_name_takes_the_wildcard() {
    // "*" is every element, not an element whose tag name is literally "*".
    let value = eval(
        "<div><p>a</p><span>b</span></div>",
        "[document.getElementsByTagName('*').length, \
          document.getElementById('d') ? 0 : document.body.getElementsByTagName('*').length].join(',')",
    );
    let (all, in_body) = value.split_once(',').expect("two counts");
    // html, head, body, div, p, span - the document form must see more than the body form.
    assert!(all.parse::<u32>().expect("all") >= 6, "document-scoped saw {all}");
    assert_eq!(in_body, "3", "body-scoped should see div, p, span");
}

#[test]
fn setting_text_content_detaches_children_rather_than_freeing_them() {
    // A test that holds a child across the assignment must still be able to read it.
    let value = eval(
        "<div id=host><span id=kid>keep</span></div>",
        "const kid = document.getElementById('kid'); \
         document.getElementById('host').textContent = 'replaced'; \
         kid.textContent + '|' + document.getElementById('host').textContent",
    );
    assert_eq!(value, "keep|replaced");
}

#[test]
fn a_repeated_id_in_one_compound_matches_nothing() {
    // #a#b is legal CSS and can never match: an element has a single id.
    let value = eval(
        "<div id=first></div><div id=second></div>",
        "String(document.querySelector('#first#second'))",
    );
    assert_eq!(value, "null");
}

#[test]
fn timer_delays_and_ids_are_coerced_like_javascript() {
    // setTimeout(f, "0") is a number after ToNumber, and clearTimeout accepts the id as a
    // string. Both answered None before, so the delay became 0 and the clear did nothing.
    let value = eval_after_timers(
        "<div></div>",
        "globalThis.log = ''; \
         setTimeout(() => { globalThis.log += 'late'; }, '20'); \
         const id = setTimeout(() => { globalThis.log += 'cancelled'; }, '10'); \
         clearTimeout(String(id));",
        "globalThis.log",
    );
    assert_eq!(value, "late");
}

#[test]
fn the_target_runs_capture_listeners_before_bubble_listeners() {
    // Registration order is bubble-then-capture; the spec invokes the target twice, so
    // capture still has to run first.
    let value = eval_after_timers(
        "<div id=target></div>",
        "globalThis.log = ''; \
         const el = document.getElementById('target'); \
         el.addEventListener('x', () => { globalThis.log += 'bubble'; }, false); \
         el.addEventListener('x', () => { globalThis.log += 'capture'; }, true); \
         el.dispatchEvent(new Event('x'));",
        "globalThis.log",
    );
    assert_eq!(value, "capturebubble");
}

#[test]
fn the_global_object_can_dispatch_to_its_own_listeners() {
    let value = eval_after_timers(
        "<div></div>",
        "globalThis.log = ''; \
         self.addEventListener('x', () => { globalThis.log += 'heard'; }); \
         globalThis.result = String(self.dispatchEvent(new Event('x')));",
        "globalThis.log + ':' + globalThis.result",
    );
    assert_eq!(value, "heard:true");
}

// ── element.style ──────────────────────────────────────────────────────────

#[test]
fn a_valid_declaration_lands_in_the_style_attribute() {
    // The block is the attribute, so writing through `style` has to be visible as markup.
    let value = eval(
        "<div id=target></div>",
        "const el = document.getElementById('target'); \
         el.style.setProperty('width', '10px'); \
         el.getAttribute('style');",
    );
    assert_eq!(value, "width: 10px");
}

#[test]
fn the_property_the_engine_rejects_is_not_set() {
    // `width: solid` parses as a declaration but means nothing for `width`, so the property
    // definition refuses it - which is what makes wpt's `test_invalid_value` meaningful.
    let value = eval(
        "<div id=target></div>",
        "const el = document.getElementById('target'); \
         el.style.setProperty('width', 'solid'); \
         el.style.getPropertyValue('width');",
    );
    assert_eq!(value, "");
}

#[test]
fn an_idl_name_reaches_its_css_property() {
    let value = eval(
        "<div id=target></div>",
        "const el = document.getElementById('target'); \
         el.style.fontSize = '12px'; \
         el.style.getPropertyValue('font-size');",
    );
    assert_eq!(value, "12px");
}

#[test]
fn a_declaration_reads_back_serialized_not_as_it_was_written() {
    // The block used to store the author's text verbatim, so every round-trip looked right and
    // none of them meant anything - `1PX` came back `1PX`. CSSOM defines `getPropertyValue` as
    // serializing the *value*, so the normalisation happens when the declaration is accepted.
    let cases = [
        // A hex colour is not a serialization; the legacy `rgb()` form is.
        ("color", "#ff0000", "rgb(255, 0, 0)"),
        // A named colour is a keyword, and stays one.
        ("color", "red", "red"),
        ("width", "1PX", "1px"),
        // `calc()` is simplified as far as it goes without an element to measure against.
        ("width", "calc(calc(100px))", "calc(100px)"),
        ("width", "calc(1in + 1px)", "calc(97px)"),
        // A percentage has no containing block here, so two terms survive - in the order
        // css-values-4 asks for, percentage before dimension.
        ("width", "calc(50px + 40%)", "calc(40% + 50px)"),
    ];

    for (property, input, expected) in cases {
        let value = eval(
            "<div id=target></div>",
            &format!(
                "const el = document.getElementById('target'); \
                 el.style.setProperty('{property}', '{input}'); \
                 el.style.getPropertyValue('{property}');"
            ),
        );
        assert_eq!(value, expected, "{property}: {input}");
    }
}

#[test]
fn a_computed_length_is_in_canonical_units() {
    // A computed value is canonical: `12cm` and a `round()` that arrives at the same length have
    // to come out the same, and they did not while only `em` and `rem` were converted here.
    let cases = [
        ("width", "12cm", "453.5433px"),
        ("width", "round(10cm, 6cm)", "453.5433px"),
        ("width", "1in", "96px"),
        ("width", "12pt", "16px"),
        // Units with nothing to resolve against travel on as written.
        ("width", "10ch", "10ch"),
    ];

    for (property, input, expected) in cases {
        let value = eval(
            "<div id=target></div>",
            &format!(
                "const el = document.getElementById('target'); \
                 el.style.setProperty('{property}', '{input}'); \
                 getComputedStyle(el)['{property}'];"
            ),
        );
        assert_eq!(value, expected, "{property}: {input}");
    }
}

#[test]
fn a_computed_value_sees_the_style_attribute() {
    // The cascade read only custom properties out of the `style` attribute - the render pipeline
    // layered the ordinary declarations on afterwards, outside the cascade - so `getComputedStyle`
    // could not see a single thing set through `element.style`. That is what wpt's
    // `test_computed_value` does before every assertion it makes.
    let value = eval(
        "<div id=target></div>",
        "const el = document.getElementById('target'); \
         el.style.width = 'calc(2em + 10px)'; \
         el.style.fontSize = '20px'; \
         getComputedStyle(el).width;",
    );
    assert_eq!(value, "50px");
}

#[test]
fn a_declaration_for_an_unknown_property_is_dropped() {
    // css-syntax-3 §9: a declaration whose property this engine does not support is invalid.
    // The cascade used to pass any property it had no definition for straight through to the
    // style map, so a misspelling was recorded as though it were a real declaration and could be
    // read back. A real property beside it still applies - an invalid declaration takes only
    // itself down.
    let value = eval(
        "<div id=target style='dsiplay: block; color: red'></div>",
        "const s = getComputedStyle(document.getElementById('target')); \
         (s.getPropertyValue('dsiplay') || 'dropped') + '|' + s.color;",
    );
    assert_eq!(value, "dropped|rgb(255, 0, 0)");
}

#[test]
fn content_is_validated_like_every_other_property() {
    // `content` used to skip validation entirely, because its grammar could not be matched
    // against the tokens the parser produced. It can now, so the exemption is gone: a value the
    // grammar accepts still applies, and one it rejects is dropped like any other.
    let value = eval(
        "<style>#a { content: \"x\" } #b { content: 10px }</style><div id=a></div><div id=b></div>",
        "const a = getComputedStyle(document.getElementById('a')).content; \
         const b = getComputedStyle(document.getElementById('b')).content; \
         a + '|' + b;",
    );
    // A dropped declaration leaves the property at its initial value.
    assert_eq!(value, "x|normal");
}

#[test]
fn attr_reads_its_type_and_its_fallback() {
    // css-values-5 §12.1: `attr( <attr-name> <attr-type>? , <declaration-value>? )`. The second
    // argument is the type, not the fallback - that comes after the comma - and an untyped
    // attr() substitutes a string, so it is only valid where a string is. This used to read the
    // type slot as the fallback and parse the attribute as a CSS value whatever was asked,
    // which let `width: attr(data-w)` take a length out of an attribute.
    let value = eval(
        "<style>\
           #a { width: attr(data-w px) } \
           #b { width: attr(missing px, 7px) } \
           #c { width: attr(data-bad px, 3px) } \
           #d { width: attr(data-w) } \
         </style>\
         <div id=a data-w=10></div><div id=b></div>\
         <div id=c data-bad=wide></div><div id=d data-w=10></div>",
        "const w = id => getComputedStyle(document.getElementById(id)).width; \
         w('a') + '|' + w('b') + '|' + w('c') + '|' + w('d');",
    );
    // The last one is invalid - a string is not a length - so `width` keeps its initial value.
    assert_eq!(value, "10px|7px|3px|auto");
}

#[test]
fn cascade_layers_sort_before_specificity() {
    // css-cascade-5 §6.4.1. A layer is a way of saying "this whole group is weak" without
    // touching specificity, so it is settled before the selectors are compared at all. Unlayered
    // CSS beats every layer, and a layer declared later beats one declared earlier. The parser
    // used to read `@layer` and throw the name away, flattening the rules into the sheet where
    // they stood, so both of these came out as plain document order.
    let unlayered_wins = eval(
        "<style>p { color: rgb(0, 128, 0) } @layer base { p { color: rgb(255, 0, 0) } }</style><p id=t>x</p>",
        "getComputedStyle(document.getElementById('t')).color;",
    );
    assert_eq!(unlayered_wins, "rgb(0, 128, 0)");

    // The order the layers were announced in wins over the order their blocks were written in.
    let announced_order = eval(
        "<style>@layer base, overrides; \
                @layer overrides { p { color: rgb(0, 128, 0) } } \
                @layer base { p { color: rgb(255, 0, 0) } }</style><p id=t>x</p>",
        "getComputedStyle(document.getElementById('t')).color;",
    );
    assert_eq!(announced_order, "rgb(0, 128, 0)");

    // A layer beats a more specific selector outside it only when the unlayered rule is weaker
    // in the cascade - which it never is. Specificity cannot rescue a layered rule.
    let specificity_does_not_help = eval(
        "<style>@layer base { p#t.c { color: rgb(255, 0, 0) } } p { color: rgb(0, 128, 0) }</style>\
         <p id=t class=c>x</p>",
        "getComputedStyle(document.getElementById('t')).color;",
    );
    assert_eq!(specificity_does_not_help, "rgb(0, 128, 0)");
}

#[test]
fn an_important_declaration_reverses_the_layer_order() {
    // The reversal is what makes layers usable for a reset: an `!important` in the *first* layer
    // wins, and an unlayered `!important` is the weakest of all (css-cascade-5 §6.4.1).
    let value = eval(
        "<style>@layer base, theme; \
                @layer base { p { color: rgb(255, 0, 0) !important } } \
                @layer theme { p { color: rgb(0, 0, 255) !important } } \
                p { color: rgb(0, 128, 0) !important }</style><p id=t>x</p>",
        "getComputedStyle(document.getElementById('t')).color;",
    );
    assert_eq!(value, "rgb(255, 0, 0)");
}

#[test]
fn a_nested_layer_sorts_inside_the_one_that_holds_it() {
    // `@layer a { @layer b { } }` is the layer `a.b`, and a layer's own rules are unlayered
    // within it, so they beat anything it nests.
    let value = eval(
        "<style>@layer a { @layer b { p { color: rgb(255, 0, 0) } } p { color: rgb(0, 128, 0) } }</style>\
         <p id=t>x</p>",
        "getComputedStyle(document.getElementById('t')).color;",
    );
    assert_eq!(value, "rgb(0, 128, 0)");
}

#[test]
fn the_style_attribute_outranks_a_layer() {
    // Element-attached styles are their own step of the cascade, above layers and specificity
    // both (css-cascade-5 §6.3). Ranking them by specificity alone was enough until layers
    // existed, because nothing else could reach that high.
    let value = eval(
        "<style>@layer base { p { color: rgb(255, 0, 0) } }</style><p id=t style='color: rgb(0, 128, 0)'>x</p>",
        "getComputedStyle(document.getElementById('t')).color;",
    );
    assert_eq!(value, "rgb(0, 128, 0)");
}

#[test]
fn revert_layer_rolls_back_only_its_own_layer() {
    // `revert-layer` asks what the property would be if this layer had said nothing, which is
    // the earlier layer's value - not the user-agent's, which is what `revert` gives. Both were
    // the same keyword here until layers existed to tell them apart.
    let value = eval(
        "<style>@layer base, theme; \
                @layer base { p { color: rgb(255, 0, 0) } } \
                @layer theme { p { color: revert-layer } }</style><p id=t>x</p>",
        "getComputedStyle(document.getElementById('t')).color;",
    );
    assert_eq!(value, "rgb(255, 0, 0)");
}

#[test]
fn a_color_keyword_computes_to_the_color_it_names() {
    // css-color-4 §15: the computed value of a colour is the colour, not the word for it. The
    // keyword is the *specified* value, which is what `element.style` reads back, and the two
    // were the same thing here - so `getComputedStyle(el).color` answered `red`.
    //
    // Whether a keyword is a colour at all depends on the property. `red` names a grid line on
    // `grid-row-start`, and must stay a name there.
    let value = eval(
        "<style>#a { color: red } #b { color: transparent } #c { grid-row-start: red }</style>\
         <div id=a></div><div id=b></div><div id=c></div>",
        "const g = id => getComputedStyle(document.getElementById(id)); \
         g('a').color + '|' + g('b').color + '|' + g('c').gridRowStart;",
    );
    assert_eq!(value, "rgb(255, 0, 0)|rgba(0, 0, 0, 0)|red");
}

#[test]
fn currentcolor_on_color_is_the_inherited_color() {
    // `currentcolor` stands for the element's own `color`, which on `color` itself means the
    // one it inherits (css-color-4 §6.2).
    let value = eval(
        "<style>#parent { color: rgb(1, 2, 3) } #child { color: currentcolor }</style>\
         <div id=parent><div id=child></div></div>",
        "getComputedStyle(document.getElementById('child')).color;",
    );
    assert_eq!(value, "rgb(1, 2, 3)");
}

#[test]
fn hwb_resolves_to_srgb_unless_a_component_is_missing() {
    // `hwb()` is a hue with white and black mixed in (css-color-4 §7). It resolves to sRGB and
    // serializes as `rgb()`, like `hsl()` - but only when every component is there. A component
    // written `none` is missing, and sRGB has no way to say that, so such a colour stays in the
    // notation it was written in.
    let value = eval(
        "<style>#a { color: hwb(120 30% 50%) } #b { color: hwb(none none none) } \
                #c { color: hwb(90deg, 50%, 50%) }</style>\
         <div id=a></div><div id=b></div><div id=c></div>",
        "const g = id => getComputedStyle(document.getElementById(id)).color; \
         g('a') + '|' + g('b') + '|' + g('c');",
    );
    // The third has commas, which `hwb()` has no legacy form for, so it is not a colour at all
    // and the declaration is dropped. What is left is `color`'s initial value, the system
    // colour `canvastext` - which stays a keyword, because the table that gives a system colour
    // a value lives in the render pipeline rather than here.
    assert_eq!(value, "rgb(77, 128, 77)|hwb(none none none)|canvastext");
}

#[test]
fn the_style_attribute_outranks_a_stylesheet_rule() {
    let value = eval(
        "<style>#target { color: red }</style><div id=target style='color: blue'></div>",
        "getComputedStyle(document.getElementById('target')).color;",
    );
    // A computed colour is the colour, not the keyword that named it.
    assert_eq!(value, "rgb(0, 0, 255)");
}

#[test]
fn an_inline_shorthand_reaches_the_computed_longhands() {
    // Inline declarations go through the same path as a stylesheet rule, so a shorthand written
    // in the attribute expands the way one in a rule does.
    let value = eval(
        "<div id=target style='margin: 1px 2px'></div>",
        "const s = getComputedStyle(document.getElementById('target')); \
         s.marginTop + '|' + s.marginRight + '|' + s.marginBottom + '|' + s.marginLeft;",
    );
    assert_eq!(value, "1px|2px|1px|2px");
}

#[test]
fn assigning_the_empty_string_removes_the_declaration() {
    // wpt's helpers clear a property this way before setting the value under test, so an
    // assignment that did nothing here would let the previous value be read back as a pass.
    let value = eval(
        "<div id=target style='width: 10px'></div>",
        "const el = document.getElementById('target'); \
         el.style.width = ''; \
         String(el.hasAttribute('style')) + '|' + el.style.width;",
    );
    assert_eq!(value, "false|");
}

#[test]
fn the_style_attribute_from_the_markup_is_read_back() {
    let value = eval(
        "<div id=target style='color: red; width: 10px'></div>",
        "const el = document.getElementById('target'); \
         el.style.getPropertyValue('color') + '|' + el.style.length;",
    );
    assert_eq!(value, "red|2");
}

#[test]
fn a_semicolon_inside_a_value_does_not_split_the_block() {
    let value = eval(
        "<div id=target style='background: url(a;b); width: 10px'></div>",
        "const el = document.getElementById('target'); \
         el.style.getPropertyValue('background-image') + '|' + el.style.getPropertyValue('width');",
    );
    assert_eq!(value, "url(\"a;b\")|10px");
}

/// `display` serializes in its short form, specified and computed alike, and an absolutely
/// positioned element computes to the block-level form (css-display-3 §2.7).
#[test]
fn display_reads_back_in_its_short_form_and_blockifies_when_positioned() {
    let value = eval(
        "<div id=target></div>",
        "const el = document.getElementById('target'); \
         el.style.display = 'inline flow-root'; \
         const a = el.style.display + '|' + getComputedStyle(el).display; \
         el.style.position = 'absolute'; \
         const b = getComputedStyle(el).display; \
         el.style.display = 'flow list-item block'; \
         a + '#' + b + '#' + el.style.display;",
    );
    assert_eq!(value, "inline-block|inline-block#block#list-item");
}

/// CSSOM §6.1: a block holds longhands. A shorthand is stored as the longhands it sets - so
/// `length` counts them, a longhand reads back, and removing one keeps the others - while the
/// shorthand itself still reads back as written for as long as it is whole.
#[test]
fn a_shorthand_is_stored_as_its_longhands() {
    let value = eval(
        "<div id=target></div>",
        "const el = document.getElementById('target'); \
         el.style.gap = '10px 20px'; \
         const a = el.style.length + '|' + el.style.item(0) + '|' + el.style.getPropertyValue('column-gap') + '|' + el.style.gap; \
         el.style.rowGap = ''; \
         const b = el.style.length + '|' + el.style.getPropertyValue('column-gap') + '|' + el.style.gap; \
         el.style.border = '1px solid red'; \
         el.style.borderTopColor = 'blue'; \
         const c = el.style.getPropertyValue('border-top-color') + '|' + el.style.getPropertyValue('border-left-color') + '|' + el.style.getPropertyValue('border-top-width') + '|' + el.style.border; \
         el.style.border = ''; \
         a + '#' + b + '#' + c + '#' + el.style.length;",
    );
    assert_eq!(value, "2|row-gap|20px|10px 20px#1|20px|#blue|red|1px|#1");
}

#[test]
fn setting_a_property_twice_keeps_its_place_in_the_block() {
    let value = eval(
        "<div id=target style='color: red; width: 10px'></div>",
        "const el = document.getElementById('target'); \
         el.style.color = 'blue'; \
         el.getAttribute('style');",
    );
    assert_eq!(value, "color: blue; width: 10px");
}

#[test]
fn style_is_the_same_object_every_time() {
    // `style` is [SameObject] in the CSSOM. A fresh proxy per access makes `el.style === el.style`
    // false and loses anything a caller hung on the object between two reads.
    let value = eval(
        "<div id=target></div>",
        "const el = document.getElementById('target'); String(el.style === el.style)",
    );
    assert_eq!(value, "true");
}

#[test]
fn a_value_cannot_inject_a_second_rule() {
    // `10px} *{color:red` closes the rule the value is spliced into and opens another. Reading
    // only the first rule would call that a valid `width: 10px` and then store the raw text in
    // the attribute, where the renderer parses it as the injected pair of rules.
    let value = eval(
        "<div id=target></div>",
        "const el = document.getElementById('target'); \
         el.style.setProperty('width', '10px} *{color:red'); \
         String(el.hasAttribute('style'));",
    );
    assert_eq!(value, "false");
}

#[test]
fn a_value_cannot_smuggle_a_second_declaration() {
    let value = eval(
        "<div id=target></div>",
        "const el = document.getElementById('target'); \
         el.style.setProperty('width', '10px; color: red'); \
         String(el.hasAttribute('style'));",
    );
    assert_eq!(value, "false");
}

#[test]
fn a_property_declared_twice_collapses_onto_the_last_value() {
    let value = eval(
        "<div id=target style='width: 10px; width: 20px'></div>",
        "const el = document.getElementById('target'); \
         el.style.getPropertyValue('width') + '|' + el.style.length;",
    );
    assert_eq!(value, "20px|1");
}

#[test]
fn custom_property_names_keep_their_case() {
    // Custom properties are case-sensitive, unlike every other CSS property name, so folding
    // them would merge `--Foo` and `--foo` into one slot.
    let value = eval(
        "<div id=target></div>",
        "const el = document.getElementById('target'); \
         el.style.setProperty('--Foo', '1'); \
         el.style.setProperty('--foo', '2'); \
         el.style.getPropertyValue('--Foo') + '|' + el.style.getPropertyValue('--foo');",
    );
    assert_eq!(value, "1|2");
}

#[test]
fn a_semicolon_inside_a_comment_does_not_split_the_block() {
    let value = eval(
        "<div id=target style='color: red /* ; */; width: 10px'></div>",
        "const el = document.getElementById('target'); \
         el.style.length + '|' + el.style.getPropertyValue('width');",
    );
    assert_eq!(value, "2|10px");
}

#[test]
fn an_escaped_quote_does_not_end_the_string() {
    // Without consuming the escape the closing quote is missed, the rest of the block is taken
    // as still inside the string, and `width` disappears from the attribute on the next write.
    let value = eval(
        "<div id=target></div>",
        "const el = document.getElementById('target'); \
         el.setAttribute('style', 'content: \"a\\\\\";b\"; width: 10px'); \
         String(el.style.getPropertyValue('width'));",
    );
    assert_eq!(value, "10px");
}

#[test]
fn css_text_replaces_the_block_and_drops_what_the_engine_refuses() {
    let value = eval(
        "<div id=target style='color: red'></div>",
        "const el = document.getElementById('target'); \
         el.style.cssText = 'width: 10px; width: solid'; \
         el.style.cssText;",
    );
    assert_eq!(value, "width: 10px;");
}
