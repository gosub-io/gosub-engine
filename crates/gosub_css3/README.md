# gosub_css3

The CSS3 system of the Gosub browser engine: tokenizer, parser, selector matcher, cascade,
and property-value validation. Parsing follows the csstree (MIT) parser design. A stylesheet
goes from text to a `CssStylesheet` (rules, selectors, declarations), and `Css3System`
implements the `CssSystem` trait from `gosub_interface` — selector matching, cascade, and
computed-value resolution for the rest of the engine.

## Entry points

- `Css3::parse_str(data, config, origin, source_url)` / `Css3::parse_stream(...)` —
  stylesheet text → `CssStylesheet`.
- `system::Css3System` — the `CssSystem` implementation; the main integration point
  (`Stylesheet = CssStylesheet`, `Property = CssProperty`, `Value = CssValue`).
- `load_default_useragent_stylesheet()` — the embedded `resources/useragent.css`.
- `matcher::syntax_matcher` — validates property values against their formal grammar
  (definitions embedded from `resources/definitions/*.json`).

## What lives here

| Module | Role |
|--------|------|
| `tokenizer`, `parser` | CSS text → AST, one parser module per construct (selectors, at-rules, calc, ...) |
| `ast`, `node` | The AST and `convert_ast_to_stylesheet` |
| `stylesheet` | The flattened `CssStylesheet` model and `CssValue` |
| `matcher` | Selector matching, cascade, shorthand expansion, value-grammar validation |
| `system` | `Css3System`, property computation, vendor-prefix normalization |
| `functions` | `var()`, `attr()`, and the math functions (`calc`, `clamp`, `min`, `max`) |
| `colors`, `walker` | Color parsing (named colors, hex, hsl, oklab/oklch) and an AST pretty-printer |

## Known limitations

- At-rule coverage stops at five. `@media`, `@supports`, `@import`, `@layer` and
  `@font-face` survive AST → stylesheet conversion; everything else is parsed and then
  dropped by `collect_rules`, including `@keyframes`, `@page`, `@scope`, `@container`,
  `@property`, `@counter-style` and `@starting-style`.
- CSS Nesting parses, but does not reach the stylesheet: `collect_rule` keeps only the
  declarations of a block, so a nested style rule is discarded rather than flattened
  into its parent's selector, and a selector carrying the nesting selector `&` is one of
  the four shapes below.
- Four selector node types have no arm in `convert_selector_children`, and a rule whose
  selector uses one is dropped (the rest of the sheet is unaffected): a bare number,
  dimension or percentage in a compound, which is how `.p-0.5` arrives once the tokenizer
  has read it as `.p-0` followed by `.5`, and the nesting selector `&`. The first three are
  invalid selectors in any engine; `&` is valid CSS Nesting this crate does not support.
- A property with no entry in the embedded definitions is treated as invalid and its
  declaration dropped, so a property from a spec the generated data does not cover is
  unsupported even when its value is well-formed.
- `calc()` terms carry a unit but no exponent, so an expression whose units only cancel
  at the end (`calc(100px * 1px / 1px)`) is left unevaluated rather than answered wrongly.
- Percentages never resolve at computed-value time - they need a containing block, which
  is layout's to know - and neither do `ch`, `lh` or the container-query units.

A recursion guard (`MAX_RECURSION_DEPTH = 64`) protects the parser against
stack-overflowing input. The `unresolved_syntax` feature exposes the definitions before
reference expansion (`get_css_values`, `get_css_properties`), in declaration order, for
tooling that needs the raw grammars rather than the resolved ones.

## Further reading

- [docs/css.md](../../docs/css.md) — the full parse → match → cascade → computed-value flow
- [docs/interface.md](../../docs/interface.md) — the `CssSystem` trait contract
- [docs/binaries.md](../../docs/binaries.md) — the `css3-parser` tool (run from the repo
  root: `cargo run --bin css3-parser`; note it parses without validating values)
