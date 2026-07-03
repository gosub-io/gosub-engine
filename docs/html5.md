# HTML5 parsing (`gosub_html5`)

The HTML5 tokenizer, tree builder, and DOM implementation. The crate implements the
`Html5Parser` and `Document` traits from [`gosub_interface`](interface.md) and follows the
WHATWG HTML parsing specification, including its error-recovery rules — invalid HTML never
fails; it produces the same tree a browser would, plus a list of recoverable
`ParseError`s.

```text
  bytes ──► ByteStream ──► Tokenizer (state machine) ──► Tokens ──► Html5Parser
                                                                    (insertion modes)
                                                                        │
                                                                        ▼
                                                            DocumentImpl (NodeArena)
```

## Tokenizer (`tokenizer/`)

A direct implementation of the spec's tokenizer state machine: `state.rs` defines one enum
variant per spec state (74 of them, each doc-commented with its spec section number, e.g.
"8.2.4.36 After attribute name state"). Supporting tables live beside it:

- `character_reference.rs` — named character references (`&amp;`, `&nbsp;`, …) with the
  spec's longest-match semantics;
- `replacement_tables.rs` — control-character and Windows-1252 code-point replacements.

The tokenizer produces `Token`s (`token.rs`: start/end tags with attributes, text, comment,
doctype, EOF) and reports errors into an `ErrorLogger` shared with the parser, so tokenizer
and tree-construction errors end up in one list with source locations.

## Tree construction (`parser.rs`)

`Html5Parser` is the spec's tree-construction stage: a loop that dispatches each token on
the current **insertion mode** (all 23 spec modes, from `Initial` through `InBody`,
the table modes, `InTemplate`, to `AfterAfterFrameset`). The spec's machinery is all here:

- the **stack of open elements** and the **list of active formatting elements** (with
  markers), including the **adoption agency algorithm** for misnested formatting tags
  (`<b><i></b></i>`);
- **foster parenting** for content that appears illegally inside tables;
- `Text` and `InTableText` buffering, pending table character tokens;
- the **template insertion mode stack** for `<template>` (whose contents parse into a
  separate fragment, exposed via `Document::template_contents`);
- **quirks-mode detection** (`quirks.rs`) from the doctype, stored on the document;
- **foreign content**: SVG and MathML get their namespaces, and `attr_replacements.rs`
  applies the spec's attribute/tag case adjustments (e.g. `viewbox` → `viewBox`);
- **fragment parsing** (`parse_fragment`, used for `innerHTML`-style parsing) with a
  context element;
- a `scripting_enabled` option (`Html5ParserOptions`) that changes how `<noscript>`
  parses, matching the spec's scripting flag.

The parser writes into the document through the small `TreeBuilder` trait
(`parser/tree_builder.rs`: create element/text/comment, insert attribute). Two
implementations exist: direct document mutation, and `DocumentTaskQueue`
(`document/task_queue.rs`), which batches mutations as `DocumentTask`s to be committed at
once — groundwork for decoupling parsing from DOM commits.

## The DOM (`document/`, `node/`)

`DocumentImpl` is the `Document` trait implementation: a `NodeArena` (a `Vec<Option<NodeImpl>>`
indexed by `NodeId`) plus document metadata (URL, doctype, quirks mode, attached
stylesheets) and a two-way index for `id="…"` lookups (`node_by_named_id` without scanning).
Node payloads live under `node/data/` (element with attributes and class list, text,
comment, doctype). As required by the `Document` trait, no `&Node` ever escapes — callers
query by `NodeId`. `writer.rs` serializes a document back to HTML (used by `Document::write`
and `inner_html`).

## Testing: the html5lib harness (`testing/`)

The crate vendors the WHATWG **html5lib-tests** suite (`tests/data/html5lib-tests/`) and
runs two of its suites:

- **tree-construction**: `testing/tree_construction/` parses the `.dat` fixture format
  (input / expected errors / expected tree), runs the full parser, serializes the resulting
  DOM with `TreeOutputGenerator`, and diffs it line-by-line against the expected tree.
  Script-on/script-off variants are honoured; a small `DISABLED_CASES` list tracks known
  failures.
- **tokenizer**: `testing/tokenizer.rs` drives the tokenizer against the JSON token
  fixtures, including tests that must start in a specific tokenizer state.

The `testing` module is explicitly allowed to panic/unwrap (see `lib.rs`) — it is test
infrastructure, where failing loudly on a malformed fixture is the right behaviour.

## Known limitations

- **Parsing is not incremental.** The whole document must be in memory before parsing
  starts (`html_compile` takes a `&str`). The TODO in `lib.rs` spells out the plan: a
  push-driven parser that accepts network chunks, so the engine can start building the DOM
  and dispatching sub-resource fetches (images, stylesheets) while the HTML response is
  still downloading.
- **No script execution during parse.** The parser tracks script nesting and pause state
  per the spec, but `document.write`-style reentrancy isn't wired to a JS engine — scripts
  are handled after parsing, not during.
