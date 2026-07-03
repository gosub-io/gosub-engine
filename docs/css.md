# CSS internals (`gosub_css3`)

How a stylesheet's text becomes the computed value the render pipeline reads for a node.
The crate implements the `CssSystem` trait from [`gosub_interface`](interface.md); its parser
is heavily based on the MIT-licensed [csstree](https://github.com/csstree/csstree) parser.

The flow has four stages:

```text
  text ──► tokenizer/parser ──► AST ──► CssStylesheet          (parse, once per sheet)
                                            │
  node ──► selector matching ──► matched declarations          (per node)
                                            │
           validation + shorthand expansion (syntax matcher)   (per declaration)
                                            │
           cascade ──► specified ──► computed ──► actual       (per property, lazy)
```

## Parsing (`tokenizer.rs`, `parser/`, `ast.rs`, `stylesheet.rs`)

The tokenizer and hand-written recursive-descent parser (one module per construct under
`parser/`: selectors, declarations, at-rules, `calc`, `an+b`, …) produce a `CssNode` AST;
`convert_ast_to_stylesheet` flattens that into the `CssStylesheet` the rest of the engine
uses: a list of `CssRule`s (selectors + declarations), plus extracted `@font-face` entries.

Every stylesheet is tagged with a `CssOrigin` — `UserAgent`, `Author` (the page's own
sheets), or `User` — which drives cascade priority later. The user-agent stylesheet ships
embedded in the crate (`resources/useragent.css`, loaded by
`load_default_useragent_stylesheet`).

## Selector matching (`matcher/styling.rs`)

`match_selector` matches one selector against one node, **right-to-left**: the rightmost
compound must match the node itself, then combinators (`>`, ` `, `+`, `~`) walk the tree
looking for matches for the remaining compounds. Pseudo-element matching is explicit: when
computing styles for `::before`/`::after`, only selectors that carry that pseudo-element
part are considered, and the rest of the compound is matched against the originating
element; conversely, a selector with a pseudo-element part never matches the element
itself.

A successful match returns a `Specificity` — the classic `(id, class, element)` triple,
compared lexicographically.

## Style collection (`system.rs::compute_properties`)

This is the orchestrator behind `CssSystem::properties_from_node`. For one node it:

1. **Filters unrenderables** — `head`/`script`/`style`/`svg`/`noscript`/`title` elements
   and whitespace-only text nodes get no property map at all (`None` = "not renderable").
2. **Collects custom properties** — a first pass walks the ancestor chain root-first,
   gathering every `--*` declaration whose selector matches, so descendants override
   ancestors. This is a simplified custom-property inheritance model (re-matching selectors
   per ancestor rather than storing computed maps).
3. **Matches every rule** in every sheet and, for each matching declaration:
   - resolves functions: `var()` against the collected custom properties, `attr()` against
     the DOM, and `clamp()`/`min()`/`max()` arithmetic (`functions/`). Resolved tokens are
     spliced *flat* into multi-token values — a nested list would break shorthand matching
     (e.g. `border: 1px solid var(--c)` must stay three top-level tokens);
   - normalizes vendor prefixes (`-webkit-x` → `x`) so values match the standard grammars;
   - looks up the property's grammar definition and validates the value (next section).
     A property *without* a definition entry is passed through unvalidated — deliberately,
     so valid-but-not-yet-defined longhands still reach consumers. `content` is also passed
     through verbatim: its grammar (strings, `attr()`, counters) isn't matcher-friendly,
     and the render pipeline resolves it itself.
4. **Applies the shorthand fix-list** — expansions collected during validation are merged
   into the map (see below).

The output `CssProperties` map holds, per property name, a `CssProperty` with the full list
of `DeclarationProperty`s that matched — value, origin, `!important`, source location, and
specificity. Nothing is decided yet; the cascade runs lazily per property.

## Validation and shorthands (`matcher/`)

`resources/definitions/*.json` (embedded at compile time) define each property's grammar in
the CSS **value definition syntax** — the notation used by the specs themselves, e.g.
`<length> | <percentage> | auto`. `matcher/syntax.rs` parses that notation with nom into a
`CssSyntaxTree` of `SyntaxComponent`s: groups with the four spec combinators
(juxtaposition, `&&` all-any-order, `||` at-least-one, `|` exactly-one), multipliers
(`?`, `*`, `+`, `{m,n}`, `#` comma-lists), literals, functions, and ~50 built-in data types
(`<length>`, `<color>`, `<ident>`, …). `matcher/syntax_matcher.rs` then matches a parsed
`CssValue` slice against that tree — this is the formal grammar validator: a declaration
whose value doesn't match its property's grammar is dropped (with a debug log), like a real
browser discards invalid declarations.

**Shorthand expansion** rides on the same match. A shorthand's definition lists its longhand
properties; while matching, a `FixList` records which matched component corresponds to which
longhand (e.g. `margin: 0 auto` → `margin-block-start: 0`, `margin-inline-start: auto`, …).
Two details matter:

- Each expansion is tagged (`FixListInfo`) with the *declaring* rule's origin, importance,
  and specificity, so an author `margin: 0` correctly outranks the UA `body { margin: 8px }`
  instead of losing on processing order.
- The `background` shorthand's full `<bg-layer>` grammar is stricter than the matcher
  supports; instead of dropping `background: url(x) no-repeat` wholesale, the system
  recovers the parts consumers understand (`background-image` from the `url()`/gradient,
  `background-color` from the color) and emits those longhands.

## The cascade and value stages (`matcher/styling.rs`)

`CssProperty::compute_value()` runs lazily (a `dirty` flag) and walks the spec's value
stages:

1. **Cascaded** — the winning declaration is the `max` of the declared list, ordered by
   origin/importance priority, then specificity. The priority ranking (per the CSS cascade
   spec): UA `!important` (7) > User `!important` (6) > Author `!important` (5) >
   Author (3) > User (2) > UA (1). Ties on both keys resolve to the *last* declaration —
   i.e. source order wins.
2. **Specified** — the cascaded value, else the inherited value (filled in by the consumer
   for inherited properties), else…
3. **Computed** — …the property's initial value from its definition.
4. **Used → Actual** — absolute units (`px`, `pt`, `in`, `cm`, `mm`, `pc`, `q`) are rounded
   to whole values; relative units (`em`, `rem`, `%`, `vw`, `vh`) are deliberately *not*
   (rounding `1.5em` to `2em` would resize headings), and `opacity` keeps its fraction
   (rounding `0.15` to `0` would make elements vanish).

Whether a property inherits, and its initial value, come from the same definitions files
(`prop_is_inherit`, `PropertyDefinition::initial_value`).

## Hover fingerprints (`system.rs`)

`hover_fingerprints` scans all sheets once and records which element types, classes, and
ids appear in a compound with `:hover` (or whether a bare `*:hover` exists). The engine
uses this to skip style recalculation entirely for pointer movement that no hover rule
could affect — and the scan lives in this crate because only the CSS system understands
its own selector representation. See the trait notes in [interface.md](interface.md).

## Known gaps

- Not every longhand has a grammar definition yet; those skip validation (by design, see
  above).
- The `background` shorthand is recovered partially (image + color; position/repeat/size
  are ignored).
- Custom-property collection re-matches selectors along the ancestor chain per node, which
  is correct but not cheap.
- At-rules are parsed into the AST, but during stylesheet conversion only two survive:
  `@font-face` (extracted into the sheet's font list) and `@layer` (its rules are flattened
  in, without layer-order cascade semantics). Everything else — including `@media` blocks
  and the rules inside them — is currently dropped.
