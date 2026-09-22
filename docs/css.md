# CSS internals (`gosub_css3`)

How a stylesheet's text becomes the computed style the render pipeline reads for a node. The
crate implements the `CssSystem` trait from [`gosub_interface`](interface.md); its parser is
heavily based on the MIT-licensed [csstree](https://github.com/csstree/csstree) parser.

The principle behind the lower half of the crate is: declare once, cascade by id, compute into a
typed struct. Everything that depends only on a declaration's text is done once per rule.
Everything that depends on the element is done per element, keyed by a generated property id
rather than a name, and its product is one typed `ComputedStyle` that layout, paint and
`getComputedStyle` all read.

``` text
  text ──► tokenizer/parser ──► AST ──► CssStylesheet               once per sheet
                                            │
              validation + shorthand expansion (syntax matcher)     once per rule, lazily
                                            │
  node ──► candidate rules (index, bloom filter) ──► match ──► cascade    per node
                                            │
           specified ──► computed (property map, by id)             per declared property
                                            │
           ComputedStyle (typed, groups shared with the parent)     per node
```

## Parsing (`tokenizer.rs`, `parser/`, `ast.rs`, `stylesheet.rs`)

The tokenizer and hand-written recursive-descent parser (one module per construct under
`parser/`: selectors, declarations, at-rules, `calc`, `an+b`, ...) produce a `CssNode` AST, which
is converted into the `CssStylesheet` the rest of the engine uses: a list of `CssRule`s
(selectors + declarations), the `@font-face` entries, the `@import` rules and the `@layer` names.

The conversion happens a rule at a time. `parse_stylesheet_streaming` hands each top-level node
to the converter as soon as it is complete and the node's subtree is dropped before the next
rule is parsed, so the AST never exists in full: on a 2.2 MB sheet it is some 460,000 nodes at
104 bytes each, and holding all of them while converting them was most of what parsing cost:
parsing that sheet went from 72 MB and 111 ms to under 30 MB and under 70 ms.
`convert_ast_to_stylesheet` still converts a whole tree for the callers that have one.

A declared value is shared, not copied. Each distinct value is allocated once per sheet
(`value_pool.rs`) and handed out as an `Arc` to every rule that writes it - of 37,706 declared
values on that sheet only 4,689 are distinct - and the same `Arc` is what each element's
property map records when the rule matches, rather than a copy of the value and its heap. The
pool's key is the value's exact representation rather than `CssValue`'s own `PartialEq`, which
for colours compares converted sRGB with a tolerance: pooling on that would replace one colour
with another a fraction of a channel away, and the survivor is what gets serialised back.

Five at-rules survive the conversion. `@media` conditions are attached to each rule inside them
and evaluated at match time against the current `MediaEnvironment` (`media_query.rs`), so a
viewport change is a restyle and never a re-parse. `@supports` asks about the engine, whose
answer cannot change while it runs, so it is settled here and a false block contributes no rules
(`supports.rs`). `@import` is recorded and resolved through a fetch callback the host supplies
(`imports.rs`), with imported rules spliced ahead of the importing sheet's. `@layer` names are
registered on the sheet and each rule carries its layer index; the order across sheets is built
per origin when the cascade needs it (`layers.rs`). Everything else is parsed and dropped.

Nothing about one rule can cost the sheet another. A selector part the converter has no arm for
invalidates that style rule and only it, a value component that does not convert invalidates its
declaration, and an at-rule that makes no sense is skipped where it stands, all per css-syntax-3
§9 and selectors-4 §3.9. The one failure that is still the whole sheet's is being handed an AST
that is not a stylesheet at all.

Every stylesheet is tagged with a `CssOrigin` (`UserAgent`, `Author`, `User`) which drives cascade
priority later. The user-agent stylesheet ships embedded in the crate (`resources/useragent.css`,
loaded by `load_default_useragent_stylesheet`).

## Validation and expansion, once per rule (`matcher/expansion.rs`)

Whether a declaration is valid and which longhands its shorthand sets depend on the declaration
alone, so `CssRule::expanded()` works that out the first time an element needs the rule and keeps
it. Each source declaration becomes one of:

-   `Custom`: a `--*` property, which cascades in a pass of its own.
-   `Invalid`: an unknown property, or a value its grammar rejects. A property with no entry in
    the definitions is unsupported and its declaration invalid (css-syntax-3 §9); nothing is
    passed through unvalidated.
-   `Pending`: the value holds `var()`, `attr()` or `light-dark()` anywhere in it, so what it
    says depends on the element. It is resolved, validated and expanded per element.
-   `Resolved`: the declaration under its own name, followed by every longhand the shorthand
    expansion produced, with the longhands a shorthand did not mention reset to their initial
    value (css-cascade-5 §2.5) and nested shorthands (`border` -> `border-color`) expanded in
    turn.

The grammars come from `resources/definitions/*.json`, embedded at compile time, written in the
specs' own value definition syntax (`<length> | <percentage> | auto`). `matcher/syntax.rs`
parses that notation with nom into a `CssSyntaxTree`; `matcher/syntax_matcher.rs` matches a
`CssValue` slice against it. Shorthand expansion rides on the same match: the matcher reports
which input values landed on which `<'longhand'>` component, a `FixList` collects them, and a
`layered` flag covers the comma-list shorthands (`background`, `transition`, `animation`) where
each longhand collects one value per layer. A few shorthands whose pieces are placed by position
rather than by grammar (`grid`, `grid-template`, `grid-area`, `background-position`, `font`) have
hand-written expanders.

## Property ids (`matcher/property_ids.rs`)

Every property the definitions know has a generated id: `LonghandId` (567), `ShorthandId` (98),
together `PropertyId`, dense and sorted by name, with static tables for the name, whether it
inherits, its initial value, whether a percentage means a number, and a shorthand's longhands.
`PropertyId::from_name` is ASCII case-insensitive, as css-syntax-3 requires of property names.
The module is generated from the JSON by `cargo run -p generate_definitions -- --property-ids`
and a test re-derives it from the embedded data, so the two cannot drift. Custom properties get
no id; they are unbounded in number and live in a map of their own.

## Style collection (`system.rs::compute_properties`)

This is the orchestrator behind `CssSystem::properties_from_node`. For one node it:

1.  **Filters unrenderables.** `head`, `script`, `style`, `svg` and `title` elements match no
    selector, and whitespace-only text nodes get no property map at all.
2.  **Gathers what the element itself carries.** Its HTML presentational hints
    (`bgcolor`, `width`, `cellspacing`, `cellpadding`), which the HTML crate maps to declaration
    text through `Document::presentational_hints`, and its `style` attribute. Both are parsed by
    the real parser into one-rule sheets, cached by their text so identical attributes share one
    sheet and one expansion.
3.  **Finds the candidate rules.** Each sheet keeps a `SelectorIndex` (`matcher/index.rs`)
    bucketing every selector by the most selective simple selector of its rightmost compound
    (id, class, type, attribute name, universal) and split by pseudo-element, so a plain element
    never looks at `::before` rules. An ancestor bloom filter (`matcher/bloom.rs`), built
    incrementally from the parent's, then drops every candidate whose ancestor compounds name an
    id, class, type or attribute no ancestor has, before the matcher walks. Both are superset
    filters: the full matcher decides.
4.  **Matches** (`matcher/styling.rs::match_selector`), right-to-left: the rightmost compound
    against the node, then the combinators walk the tree, backtracking through descendant and
    sibling combinators. A rule inside `@media` is checked against the environment first, since
    that is cheaper than matching. A rule applies with the highest specificity among its matching
    selectors. Pseudo-element requests (`::before`, `::after`) consider only selectors carrying
    that pseudo-element and match the rest against the originating element.
5.  **Cascades custom properties** in a pass of their own, over the parent's scope. The parent's
    map is shared by `Arc` unless the element actually changes a value: on a utility-framework
    page that resets fifty `--*` on `*`, copying them per element was once the dominant cost.
6.  **Records the declarations.** For a `Resolved` declaration every entry is pushed with the
    element's cascade facts; for a `Pending` one the substitution functions are resolved first,
    wherever they stand, function arguments included (css-variables-1 §3), the value is reduced
    the way the parser reduces one, then validated and expanded. An unresolvable reference with
    no fallback drops the declaration.
7.  **Resolves the font-size basis**, so `em` and `rem` have a value to resolve against.

## The cascade (`matcher/styling.rs`)

The output map (`CssProperties`) holds one `CssProperty` per declared property, keyed by id in
a slot table, each with the full list of `DeclarationProperty`s that reached it. The cascaded
value is their `max` under one comparison chain, read top to bottom, first difference wins:

1.  origin and importance: UA `!important`, user `!important`, author `!important`, then
    author, user, UA (css-cascade-5 §6.1);
2.  tree rank: shadow depth, shallower wins for normal declarations and deeper for important
    ones (§6.2);
3.  the element's own `style` attribute (§6.3);
4.  layer rank: unlayered is the top of the order for a normal declaration and the bottom for an
    important one, and the layers run in opposite directions for the two (§6.4);
5.  specificity;
6.  document order, a running counter across the matched rules. Presentational hints are
    normal, unlayered author-origin declarations with zero specificity that take the lowest
    orders, so they beat the user-agent sheet and, being unlayered, beat author declarations
    inside an `@layer`; any unlayered author declaration beats them on specificity or order,
    and any important declaration outranks them. A longhand produced by a shorthand inherits
    the shorthand's order.

`revert` and `revert-layer` are not values: they re-run the cascade over the declarations that
remain once the winning origin or layer is removed.

## The value stages

`CssProperty::compute_value()` walks the spec's stages once per property:

1.  **Cascaded**, as above.
2.  **Specified**: the cascaded value, or what the property falls back to. `inherit` and `unset`
    name the inherited value, which comes from an `InheritedValues` chain: each element records
    the computed values it settled itself, once, and shares that record with every child, so
    "what does this element inherit for x" is a short walk up rather than a copy into every map.
3.  **Computed**: `initial` becomes the property's initial value; a colour keyword becomes the
    colour it names, and a colour function is folded; `font-size` percentages resolve against the
    parent; `thin`/`medium`/`thick` become lengths; every unit with a known conversion becomes
    canonical (`px`, `deg`, `s`) with `em` and `rem` measured against the element's and the
    root's font-size; math functions are simplified; and the property's range clamps the result
    (css-values-4 §10.12). Percentages, `ch`, `lh` and the container-query units survive as
    written, because nothing here has a value for them.

Only the computed value is kept. The cascaded and specified values are steps on the way to it,
read by nothing but that walk, so they are threaded through as locals rather than stored on every
declared property of every element; `cascaded_value()` and `specified_value()` recompute them for
the style dump, which asks once per property per run.

There is no used or actual stage in this crate. Those belong to layout.

## The typed style (`matcher/computed_style.rs`, `gosub_interface::style`)

`CssPropertyMap::computed_style(map, parent)` turns the map into the `ComputedStyle` the
pipeline reads: eleven field groups (inherited: font and text; reset: box, size, margin, padding,
border, outline, background, inset, flex, grid), each behind an `Arc`. A group the element
declared nothing in shares the parent's `Arc` (inherited groups) or the process-wide initial
(reset groups); border is the exception, since its colours default to `currentColor`, and shares
the parent's only when the parent declared no border property either. A `DeclaredSet` bitset
records which properties the element's own cascade produced, for the readers that ask "did the
author set this". Percentages travel on as `LengthPercentage` and layout resolves them.

Some of what this conversion does belongs in the computed stage above and is listed under the
gaps: it is where the system colours, the `font-size` keyword scale, the `ch`/`ex`/`lh`/`ic`
approximations, `currentColor` on a property other than `color`, `outline-color: auto` and the
physical-to-logical inset mapping are still decided.

## Hover fingerprints (`system.rs`)

`hover_fingerprints` scans all sheets once and records which element types, classes, and ids
appear in a compound with `:hover` (or whether a bare `*:hover` exists). The engine uses this to
skip style recalculation entirely for pointer movement that no hover rule could affect, and the
scan lives in this crate because only the CSS system understands its own selector
representation. See the trait notes in [interface.md](interface.md).

## Measuring

Two Criterion benchmarks cover this crate. `css_parser` in `crates/gosub_css3/benches` times the
tokenizer and the parser over the user-agent sheet and the 2.2 MB real-world sheet in
`tests/data/css3-data`. `style` in `crates/gosub_render_pipeline/benches` times what happens after
parsing: giving every element its computed style. It has to live in the pipeline crate because
that is the one place that can reach the HTML parser, this crate and the adapter together.

`style` reports two numbers per fixture. `cascade` walks the element tree top-down through
`Css3System` alone, computing every property, which is this crate's cost in isolation.
`render-tree` builds a cold adapter and the render tree over it, which adds the conversion from
the property map into the typed `ComputedStyle` the pipeline reads. The fixtures are a
dozen-element floor, a generated utility-first page of about 3,000 class-heavy elements with
several thousand rules, and the wikipedia fixture DOM under the 2.2 MB sheet. Throughput is
reported per styled element.

Read `cascade` as the control when judging a `render-tree` number: it is the same code on both
sides of a pipeline-only change, so whatever it reports is the machine's own drift between the
two runs, which on a shared workstation is several percent. Take the numbers that decide
anything on a quiet, dedicated machine, back to back, with a target directory per source tree.

To compare a change against the code before it, save a baseline first and name it:

``` bash
cargo bench -p gosub_render_pipeline --bench style -- --save-baseline before
# make the change
cargo bench -p gosub_render_pipeline --bench style -- --baseline before
```

## Memory

`memory_dump` (`cargo run --release -p gosub_render_pipeline --example memory_dump`) prints
where a page's memory goes, row by row: the DOM, the parsed sheets, the selector index, and the
per-element property maps and computed styles. Shared allocations are counted once, on the first
row that reaches them, and kept in their own column - anything else would price the `Arc`-shared
style groups and pooled values once per sharer, which is the number this crate was rebuilt to
make false. The report reads `VmRSS` from `/proc/self/status` as well, so it states what
fraction of the page's real cost it accounts for rather than leaving that to trust, and it
asserts it never claims more than the process occupies.

`parse_peak` (`cargo run --release -p gosub_css3 --example parse_peak`) is the narrower tool: it
parses the 2.2 MB sheet and reports the time, the resident memory it costs, and how many of its
declared values are repeats.

Where the memory goes, on the wikipedia fixture under that sheet: the largest single row is the
declarations each element cascaded, then the parsed sheets themselves. What an element's
computed values cost is almost nothing - a few bytes each, because a computed value is usually a
number and a unit with nothing on the heap. Two things dominate what is left and neither is a
byte-packing problem: every declaration that reached a property is kept, losers included, so
that `revert` can ask what the cascade would have said without an origin, and each element pays
for a property slot table sized to every property the engine knows.

## The correctness gate

Two examples in the pipeline crate are the correctness gate for any change here.
`style_dump` writes every element's property map, declared and computed, for 30 fixture pages;
`render_dump` writes the layout geometry and paint commands for the same pages. A change that
should not alter output is checked by diffing both before and after; a change that should, such
as a spec fix, is checked by attributing every differing line to the element, the declaration
and the clause that makes the new value right.

## Known gaps

-   A pseudo-class with an argument is stored as its text and never matches: `:nth-child()` and
    the rest of the nth family, `:is()`, `:where()`, `:has()`, `:lang()`. Only `:not()` is
    evaluated, and the specificity of `:is()` and `:has()` errs low for the same reason.
    `:first-child` and `:last-child` count text nodes as siblings. `:active` is hardcoded false;
    `:visited` deliberately so.
-   Four selector node types have no conversion arm, so a rule using one is dropped: a bare
    number, dimension or percentage in a compound (`.p-0.5` tokenizes as `.p-0` and `.5`, and
    all three are invalid selectors anyway) and the nesting selector `&`. CSS Nesting parses but
    its nested style rules are discarded by `collect_rule`.
-   At-rule coverage stops at the five above.
-   An unresolvable `var()` drops its declaration; css-variables-1 §3.1 says the property should
    then compute to `inherit` or `initial`.
-   The six computed-value questions still answered in the typed-style conversion (listed above).
    One consequence: a relative `font-size` keyword (`smaller`) travels down as the keyword and
    is re-applied on every descendant.
-   `calc()` terms carry a unit but no exponent, so an expression whose units only cancel at the
    end (`calc(100px * 1px / 1px)`) is left unevaluated rather than answered wrongly.
-   Percentages, `ch`, `lh` and the container-query units never resolve at computed-value time.
-   Presentational hints are unlayered, so they beat author rules inside an `@layer`; whether that
    is what css-cascade-5 means by "as if at the start of the author style sheet" is undecided.
-   `<svg>` is on the unrenderable list, so no selector reaches an SVG element.
-   Subtree invalidation on restyle compares only custom properties, so an inherited change
    through `:hover` can leave descendants stale.
