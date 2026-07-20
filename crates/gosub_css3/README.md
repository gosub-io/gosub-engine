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

- Most at-rules are dropped during AST → stylesheet conversion; only `@font-face` and
  `@layer` survive (the latter without layer-cascade semantics), and `@media` is not
  yet honored.
- Not every longhand has a value-grammar definition, and the `background` shorthand is
  only partially recovered.

A recursion guard (`MAX_RECURSION_DEPTH = 64`) protects the parser against
stack-overflowing input. The `unresolved_syntax` feature gates experimental
syntax-matcher paths.

## Further reading

- [docs/css.md](../../docs/css.md) — the full parse → match → cascade → computed-value flow
- [docs/interface.md](../../docs/interface.md) — the `CssSystem` trait contract
- [docs/binaries.md](../../docs/binaries.md) — the `css3-parser` tool (run from the repo
  root: `cargo run --bin css3-parser`; note it parses without validating values)
