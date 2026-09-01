# Binary reference

Every runnable binary in the repository, in one place. Each binary prints a
one-line banner (name, version, purpose) on stderr when it starts.

| # | Binary | Run with | What it does |
|---|--------|----------|--------------|
| 1 | `css-check` | `cargo run --bin css-check` | Parse a CSS file or URL with `ignore_errors` on; every rule the parser cannot handle is logged as a warning, then the total rule count is printed. |
| 2 | `css3-parser` | `cargo run --bin css3-parser` | Parse a CSS resource and pretty-print the stylesheet tree, or an annotated error snippet. Flags for a raw tokenizer dump and property value-syntax matching. |
| 3 | `display-text-tree` | `cargo run --bin display-text-tree` | Fetch a URL, parse the HTML and print only the text nodes — a quick check on what the parser sees, with no layout or styling. |
| 4 | `gosub-parser` | `cargo run --bin gosub-parser` | Fetch and parse an HTML page, then report discovered stylesheets, parse errors and timing statistics. |
| 5 | `html5-parser-test` | `cargo run --bin html5-parser-test` | Run the html5lib tree-construction fixture suite (`*.dat`) and print a compact pass/fail summary. |
| 6 | `parser-test` | `cargo run --bin parser-test` | html5lib tree-construction dev harness with detailed per-test failure output; accepts fixture filenames as arguments to filter. |
| 7 | `run-js` | `cargo run --bin run-js` | Execute a JavaScript file in the bare V8 engine (no DOM or Web APIs) and print the resulting value. |
| 8 | `gosub-screenshot` | `cargo run -p gosub-screenshot` | Headless full-page screenshot: load a URL through the complete engine + render pipeline and write a PNG. CPU rasterization only — no GPU, no window. See [headless.md](headless.md). |
| 9 | `table_console` | `cargo run -p gosub_lattice --bin table_console` | Console demos of the lattice table layout engine (colspan/rowspan/section clamping) rendered as ASCII tables; mirrors the integration tests in `gosub_lattice/src/tests.rs`. |
| 10 | `generate_definitions` | `cargo run -p generate_definitions` | Regenerate the CSS property/value definition JSON embedded in `gosub_css3` (`resources/definitions/`) by merging webref spec grammars with MDN metadata. |
| 11–20 | GUI example apps | `cargo run -p example-<name>` | Ten browser-shell binaries (`egui`/`gtk4`/`winit` × `cairo`/`skia`/`skia-gpu`/`vello`) that open a window and drive `GosubEngine` with the named backend. See [examples.md](examples.md). |
| 21 | `css3_parser` (fuzz) | `cargo +nightly fuzz run css3_parser` (from `crates/gosub_css3/fuzz`) | libFuzzer target feeding arbitrary bytes to the CSS3 parser. No startup banner — libFuzzer owns `main`. |
| 22 | `html5_parser` (fuzz) | `cargo +nightly fuzz run html5_parser` (from `crates/gosub_html5/fuzz`) | libFuzzer target for the HTML5 tree-construction parser. |
| 23 | `tokenizer` (fuzz) | `cargo +nightly fuzz run tokenizer` (from `crates/gosub_html5/fuzz`) | libFuzzer target for the HTML5 tokenizer. |

Besides these, the workspace ships `cargo run --example …` targets (hello-world,
multi-tab, tutorial, config-store, …); see [examples.md](examples.md).

------------------------------------------------------------------------

# Component tool reference

These binaries each exercise a single crate in isolation --- the HTML5 parser, CSS3 parser, etc. They are useful for development and debugging but are not the primary way to drive the engine.

To see the full `GosubEngine` stack in action (multi-zone/tab model, async networking, event bus), run the engine examples instead:

```bash
cargo run --example hello-world    # single tab, headless
cargo run --example multi-tab      # 25 tabs, live progress bars
cargo run -p example-gtk4-cairo    # GTK4 window
cargo run -p example-egui-vello    # egui/wgpu window
```

See [`examples/README.md`](../examples/README.md) for details.

## css-check

Parse a single CSS source (local file or `http(s)://` URL) and report the result. The parser runs with `ignore_errors` enabled, so it does not stop at the first problem; every rule it cannot parse is emitted as a `WARN` log line (the same warnings you see in the renderer examples). Set `RUST_LOG=trace` for deeper inspection.

```bash
$ cargo run -r --bin css-check https://example.com/style.css

Parsed https://example.com/style.css: 12 rule(s).
```

## css3-parser

Parse a CSS stylesheet and print the parse tree (or any errors encountered). `--match-values` additionally checks each declaration value against the property's grammar; `--tokenizer` dumps raw tokens instead of parsing.

```bash
$ cargo run -r --bin css3-parser file://tests/data/css3-data/test.css

Running css3 parser of (54.00 B) took 0 ms.
[Stylesheet (1 rules)]
  [Rule]
    [SelectorList (2)]
      [Selector]
        [Type] div
      [Selector]
        [Type] a
    [Block (2 declarations)]
      [Declaration] color
        String("white")
      [Declaration] border
        List([Unit(1.0, "px"), String("solid"), String("black")])
```

## gosub-parser

Fetch a URL, parse the HTML5 and any linked CSS, then print parse errors and timing statistics.

```bash
$ cargo run -r --bin gosub-parser https://news.ycombinator.com

Parsing url: Url { scheme: "https", ... host: Some(Domain("news.ycombinator.com")), ... }

Found 1 stylesheets
Stylesheet location: "https://news.ycombinator.com/news.css?..."

Parse Error: expected-doctype-but-got-start-tag
Parse Error: link element with rel attribute 'icon' is not supported in the body
...

Namespace            |    Count |      Total |        Min |        Max |        Avg
------------------------------------------------------------------------------------
html5.parse          |        1 |      605ms |      605ms |      605ms |      605ms
decode.css           |        1 |      613µs |      613µs |      613µs |      613µs
```

## display-text-tree

Fetch a URL and print a plain-text representation --- all text nodes from the parsed document, with no layout or styling applied. Useful for a quick sanity check on what the parser sees.

```bash
$ cargo run -r --bin display-text-tree https://gosub.io
```

## html5-parser-test

Run the html5lib tree-builder test suite from the command line. The test data files must be reachable from the working directory; run from the repo root.

```bash
$ cargo run -r --bin html5-parser-test
```

## parser-test

A focused parser development harness for running specific HTML5 parser tests during development, with detailed per-test failure output. Pass fixture filenames as arguments to run a subset. Intended for use while actively working on the parser; not a substitute for the full test suite.

```bash
$ cargo run -r --bin parser-test
```

## run-js

Run a JavaScript file through the V8 engine. There is no DOM or Web API binding, so browser globals (`console`, `document`, `fetch`, etc.) are not available.

```javascript
var a = 1 + 3
a
```

```bash
$ cargo run -r --bin run-js tests/example1.js
Got Value: 4
```

## table_console

Console renderer for `gosub_lattice`, the table layout engine. Each demo (plain grid, colspan, rowspan, rowspan clamped at section boundaries) mirrors an integration test in `crates/gosub_lattice/src/tests.rs`, so the scenarios the tests assert numerically can be eyeballed as ASCII tables.

```bash
$ cargo run -p gosub_lattice --bin table_console
```

## generate_definitions

Regenerates the CSS definition JSON files embedded in `gosub_css3` (`resources/definitions/`) by downloading and merging webref's spec grammars with MDN's property metadata. Output lands in `.output/definitions`; see the tool's own README for the full data-flow description.

```bash
$ cargo run -p generate_definitions
```

## Fuzz targets

Three libFuzzer targets live outside the workspace and are built with [`cargo fuzz`](https://github.com/rust-fuzz/cargo-fuzz) (nightly):

```bash
cd crates/gosub_css3/fuzz  && cargo +nightly fuzz run css3_parser
cd crates/gosub_html5/fuzz && cargo +nightly fuzz run html5_parser
cd crates/gosub_html5/fuzz && cargo +nightly fuzz run tokenizer
```

## config-store (example, not a binary)

View and modify the configuration store. Note this is an *example* target: run it with `--example`, not `--bin`.

```bash
$ cargo run -r --example config-store list

dns.cache.max_entries                   : u:1000
dns.cache.ttl.override.enabled          : b:false
dns.local.enabled                       : b:true
useragent.default_page                  : s:about:blank
useragent.tab.max_opened                : i:-1
...

$ cargo run -r --example config-store search --key 'user*'

useragent.default_page                  : s:about:blank
useragent.tab.close_button              : m: left
useragent.tab.max_opened                : i:-1
```
