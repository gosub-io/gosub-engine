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
  (`Stylesheet = CssStylesheet`, `PropertyMap = CssProperties`, `Property = CssProperty`,
  `Value = CssValue`).
- `CssPropertyMap::computed_style(map, parent)` — the one conversion from a property map into
  the typed `ComputedStyle` (`gosub_interface::style`) that layout, paint and
  `getComputedStyle` read.
- `matcher::property_ids::PropertyId` — the generated id of every known property, with its
  static tables; `from_name` is the only name lookup the cascade does.
- `load_default_useragent_stylesheet()` — the embedded `resources/useragent.css`.
- `matcher::syntax_matcher` — validates property values against their formal grammar
  (definitions embedded from `resources/definitions/*.json`).

## What lives here

| Module | Role |
|--------|------|
| `tokenizer`, `parser` | CSS text → AST, one parser module per construct (selectors, at-rules, calc, ...) |
| `ast`, `node` | The AST, and the conversion into a stylesheet - a rule at a time as the parser finishes it, so the tree never exists in full |
| `stylesheet` | The flattened `CssStylesheet` model, `CssRule` with its lazily expanded declarations, and `CssValue` |
| `value_pool` | One allocation per distinct declared value in a sheet, shared with every rule that writes it and every element those rules match |
| `media_query`, `supports`, `imports`, `layers` | The five at-rules that survive conversion: `@media` evaluated at match time, `@supports` settled at build time, `@import` resolved through a host callback, `@layer` ordered per origin |
| `matcher::syntax`, `matcher::syntax_matcher`, `matcher::property_definitions` | The value definition syntax parser, the grammar matcher, and the embedded definitions |
| `matcher::expansion`, `matcher::shorthands` | Validation and shorthand expansion, done once per rule |
| `matcher::property_ids` | Generated `LonghandId`/`ShorthandId`/`PropertyId` and their static tables |
| `matcher::index`, `matcher::bloom` | The selector index (by rightmost compound, split by pseudo-element and attribute) and the ancestor bloom filter; both superset filters ahead of the matcher |
| `matcher::styling` | Selector matching, the cascade, the value stages, the property map and the inheritance chain |
| `matcher::computed_style` | The property map to typed `ComputedStyle` conversion |
| `system` | `Css3System` and `compute_properties`, the per-element orchestrator; presentational hints and the `style` attribute as cached one-rule sheets; hover fingerprints |
| `functions` | `var()`, `attr()` and `calc()` arithmetic, with substitution reaching into function arguments |
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

- [docs/css.md](../../docs/css.md) — the full parse → match → cascade → computed-value flow,
  the gates and the benchmarks
- [docs/css_properties.md](docs/css_properties.md) — values, the value definition syntax and
  where the definitions come from
- [tools/generate_definitions/README.md](tools/generate_definitions/README.md) — regenerating
  the definitions and the property ids
- [docs/interface.md](../../docs/interface.md) — the `CssSystem` trait contract
- [docs/binaries.md](../../docs/binaries.md) — the `css3-parser` tool (run from the repo
  root: `cargo run --bin css3-parser`; note it parses without validating values)
