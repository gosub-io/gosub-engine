# gosub_html5

The HTML5 tokenizer and parser of the Gosub browser engine, plus the DOM it produces. The
tokenizer implements the WHATWG state machine (74 states, character references, error
recovery), the tree builder implements the 23 insertion modes, and the result is a
`DocumentImpl` — an arena-backed DOM addressed by `NodeId` handles, implementing the
`Document` trait from `gosub_interface`.

## Entry points

- `Html5Parser::parse_document(&mut stream, &mut doc, options)` — parse a full document
  from a `ByteStream`; `parse_fragment(...)` for the fragment case.
- `html_compile::<C>(html)` — one-call convenience: string in, document out.
- `DocumentBuilderImpl::new_document(url)` — create an empty document to parse into.
- `writer` — serialize a document back to HTML.

## What lives here

| Module | Role |
|--------|------|
| `tokenizer` | WHATWG tokenizer state machine, tokens, named character references |
| `parser` | Tree construction: insertion modes, quirks handling, foreign content |
| `document` | `DocumentImpl` + `NodeArena` storage, builder, fragments, task queue |
| `node` | `NodeImpl` node payloads, arena, element data, visitors |
| `writer` | DOM → HTML serialization |
| `testing` | Harness for the vendored WHATWG html5lib-tests suite |

## Features and testing

- Cargo features `debug_parser` / `debug_parser_verbose` gate extra parser tracing.
- Conformance tests run against the vendored html5lib-tests suite in `tests/data/`;
  the `html5-parser-test` and `parser-test` binaries live in the root package, so run
  them from the repo root: `cargo run --bin html5-parser-test`.
- Criterion benches: `tokenizer`, `tree_construction`, `html_parser` (the latter installs
  a counting allocator, which is why this crate relaxes the workspace `unsafe_code` lint).

## Further reading

- [docs/html5.md](../../docs/html5.md) — tokenizer/tree-builder structure and the
  html5lib test harness
- [docs/binaries.md](../../docs/binaries.md) — the `gosub-parser`, `html5-parser-test`
  and `display-text-tree` component tools
- [docs/interface.md](../../docs/interface.md) — the `Html5Parser` / `Document` trait
  contracts this crate implements
