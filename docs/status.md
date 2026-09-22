# What the engine can do today

One section per component: what works, and what does not work yet. The README's Status list is
the short version.

Measured on 2026-09-22 against `new-css-delcaration-design`. Where a number can be regenerated,
the command is next to it. Anything without a command is a reading of the code and will drift.

1,547 tests pass, none fail (`cargo test --workspace`).

The CSS parser validates 666 properties against generated grammars. Only 92 reach layout and
paint. Accepting a declaration and acting on it are different things, so the sections below say
which of the two they mean.


## HTML5 parser - `gosub_html5`

**Works.** 3,313 of 3,317 html5lib tree-construction tests pass - **99.88%**
(`cargo run --bin html5-parser-test`). That is the full spec tokenizer state machine, the
insertion-mode tree builder, foreign content (SVG and MathML), the adoption agency algorithm,
fragment parsing via `parse_fragment`, character references, and the error positions html5lib
checks. Shadow DOM: attachment, the sealed root, flat-tree traversal, and 21 dedicated tests.
The arena DOM behind it is what the rest of the engine reads.

**Not yet.** The parser is one-shot: it takes a complete byte stream and returns a document.
There is no incremental or push-driven mode, so a caller cannot feed it chunks as they arrive
off the network. `document.write` is not implemented. UTF-16 private-use surrogates are not
handled. The html5lib suite parses whole documents, so it covers none of this.


## CSS - `gosub_css3`

**Works.** The parser covers the css-syntax grammar and validates declarations against property
definitions generated from webref merged with MDN - 666 properties, plus selectors and at-rules
(`crates/gosub_css3/resources/definitions/`). Selector matching is right-to-left and backtracks
through descendant and sibling combinators. The cascade handles origins, specificity,
`!important`, and the shadow-tree tiebreak. Custom properties and `var()` resolve, including
fallbacks and nesting. Shorthands expand and reset the longhands they do not mention. The math
functions are essentially complete: `calc()`, `min`/`max`/`clamp`, `round`/`mod`/`rem`, the trig
family, `abs`/`sign`, `pow`/`sqrt`/`hypot`, `exp`/`log`, evaluated with correct type rules and
range clamping at computed-value time. `@media` conditions evaluate, `calc()` included.
`var()`, `attr()` and `light-dark()` substitute wherever they stand, inside another function's
arguments included. HTML presentational hints (`bgcolor`, `width`, `cellspacing`, `cellpadding`)
cascade as author-level, specificity-zero declarations ahead of the author sheets.

The bottom half of the crate was rebuilt in September 2026. A declaration is validated and its
shorthand expanded once per rule, not once per matched element (`matcher/expansion.rs`). The
cascade is keyed by generated property ids (`matcher/property_ids.rs`, regenerated with
`cargo run -p generate_definitions -- --property-ids`). Its product is one typed `ComputedStyle`
(`crates/gosub_interface/src/style.rs`), built once per element by `matcher/computed_style.rs`
with its field groups shared with the parent where nothing was declared, and read directly by
layout, paint and `getComputedStyle`. Inherited values travel by a chain, not by copying into
every child. Candidate rules come from a selector index split by pseudo-element and attribute
name, and an ancestor bloom filter drops the selectors whose ancestors an element does not have
before the matcher walks. On the benchmark fixtures (`cargo bench -p gosub_render_pipeline
--bench style`, measured on a dedicated Apple M1) the cascade on a 1,225-element page under an
18,537-rule real-world sheet went from 183 ms to 21 ms, and the five-stage pipeline on the
wikipedia fixture from 246 ms to 84 ms. The dev tools `style_dump` and `render_dump`
(`cargo run -p gosub_render_pipeline --example ...`) write every element's property map and
the layout and paint output for 30 fixture pages; every step of the rebuild was gated on those
being byte-identical, or on each difference being attributed to a spec clause.

Memory was the second half of the work, measured with `memory_dump`
(`cargo run --release -p gosub_render_pipeline --example memory_dump`), which reports where a
page's memory goes with shared allocations counted once and states what fraction of the page's
real cost the rows account for. On the wikipedia fixture under the 18,537-rule sheet, peak
resident memory fell from 152.6 MB to about 55 MB. Five changes account for it, and none of them
altered a byte of either dump. The CSS property definitions used to occupy 60 MB before a page
was parsed, because resolution inlined each named type into every property referencing it and
into every other type: the resolved types are now shared behind an `Arc` and the named-type map
is freed once resolution is done, taking that to 3.7 MB and making the cascade faster, since the
grammar now fits in cache. Parsing a sheet used to build the whole AST before converting any of
it; each top-level rule is now converted as it is parsed and its nodes dropped, which took
parsing the document and its stylesheets from 72.9 MB to 21.5 MB, and the 2.2 MB sheet alone
from 111 ms to under 70 ms. A declared value is shared
with every element its rule matches rather than deep-copied into each one, and identical values
are pooled across the rules of a sheet (`value_pool.rs`). `CssColor` packs its components rather
than holding four `Option<f64>`, which shrank `CssValue` - the largest variant of which is a
colour - from 72 bytes to 48, and with it every declaration and property in the engine. A
selector list is one allocation rather than four.

Against Chromium 152 loading the same DOM and sheet headless, measured as the renderer's
proportional set size above a blank page, the engine costs roughly three times as much. The gap
is no longer in what a value or a selector costs: it is that every element keeps the
declarations that reached it, losers included, where a browser applies them and drops them.

**Not yet.** A pseudo-class with an argument is stored as its text and never matches:
`:nth-child()` and the rest of the nth family, `:is()`, `:where()`, `:has()` and `:lang()` all
fall to the matcher's catch-all (`crates/gosub_css3/src/matcher/styling.rs`); only `:not()` is
evaluated, and the specificity of `:is()` and `:has()` errs low for the same reason.
`:first-child` and `:last-child` count text nodes as siblings. `:active` is hardcoded false.
`:visited` is hardcoded false to avoid leaking browsing history. CSS Nesting's `&` selector is
the one selector form the AST converter has no arm for, so a nested rule is dropped. An
unresolvable `var()` drops its declaration where css-variables-1 says the property should compute
to `inherit` or `initial`. Six computed-value questions are still answered when the typed struct
is built rather than in the crate's computed stage - system colours, the `font-size` keyword
scale, `ch`/`ex`/`lh`/`ic`, `currentColor` on a property other than `color`, `outline-color:
auto` and the physical-to-logical inset mapping - so `getComputedStyle` reports the unresolved
form for those, and a relative `font-size` keyword is re-applied on every descendant. A hovered
element whose inherited `color` changes leaves its descendants' cached styles stale: subtree
invalidation compares only custom properties. `<svg>` is on the list of elements that never get
a computed style, so no selector reaches one. `getComputedStyle` exists and is cached per
document; the CSS WPT component stands at 3,349 of 5,759 subtests
(`WPT_ROOT=... make wpt-css`), the property-parsing suites at 15,543 of 22,832
(`make wpt-css-parsing`).


## Layout and paint - `gosub_render_pipeline`

**Works.** 92 CSS properties reach layout and paint. The list is the fields of `ComputedStyle`
in `crates/gosub_interface/src/style.rs` - grep that for the current set. It
covers the box model, flex, grid (including `grid-template-areas`), floats with document-order
band resolution, absolute and fixed positioning, `position: sticky`, overflow and scrolling,
borders and radii, backgrounds and gradients, outlines, opacity and `mix-blend-mode`, and the
text properties. Block layout, flex and grid come from Taffy; inline content is emulated with
anonymous flex containers; CSS tables go to `gosub_lattice` through the `TableTree` adapter.
After layout: paint, layer promotion, tiling, rasterization, and compositing.

**Not yet.** No CSS transforms beyond `translate` - the rest of the `transform` list is parsed
and then ignored. No transitions or animations; nothing in the pipeline reads either property.
No writing modes, so vertical text and RTL block flow are absent (the logical `inset-*`
properties exist, but only because physical `top`/`left`/`right`/`bottom` are mapped onto them -
see `resolve_insets` in `crates/gosub_css3/src/matcher/computed_style.rs`).


## Render backends

**Works.** Null (headless), Cairo (CPU, Pango text), Skia (CPU and GPU via OpenGL), Vello
(GPU via wgpu), and a dynamic backend that picks at runtime. Each has its own example across
three window toolkits - winit, GTK4 and egui - and `bin/gosub-screenshot` renders full pages
without a window or a GPU.


## Fonts - `gosub_fontmanager`

**Works.** Two backend families that stay consistent through one shared font collection: font
systems that measure text for layout (Pango, Parley, Skia) and text rasterizers that draw
glyphs. Web fonts load through the resource pipeline, so `@font-face` works
(`engine/resource_pipeline/webfonts.rs`).


## Images, SVG and media

**Works.** Raster images decode through the `image` crate's default format set, with a lenient
PNG retry for files whose checksums other browsers also ignore. SVG parses to a `usvg::Tree`,
either from a standalone document or from an `<svg>` subtree of an HTML document, and the
backends rasterize it.

**Not yet.** Still frames only: an animated GIF renders its first frame. There is no audio or
video - `TabCommand::PlayMedia` and `PauseMedia` are declared in the API and not handled by the
tab worker.


## Networking - `gosub-sonar` 0.7 (external crate)

The networking stack lives in its own project and is consumed from crates.io, where it carries
its own documentation.

**Works.** Four priority lanes with global and per-origin connection limits, coalescing of
identical in-flight requests with fan-out and per-subscriber cancellation, buffered or streamed
bodies with automatic decompression and timeouts. The web platform policies are applied on every
redirect hop: CORS with preflights and response tainting, referrer policy, mixed content,
`Sec-Fetch-*`, HSTS, and credential stripping across origins. HTTP caching per RFC 9111 with
`ETag`/`Last-Modified` revalidation and `Vary`. Auth challenges, pluggable DNS for SSRF
policies, proxies, TLS error overrides. It also compiles for wasm on top of the browser's
`fetch()`. Inside the engine, `http`, `https`, `data` and `file` URLs all resolve, and there is
a decision hub for asking the embedder about TLS failures and downloads.

**Not yet.** No HTTP/3 or QUIC. No WebSockets - `ResourceKind::WebSocket` exists as a
classification and nothing implements it.


## Process isolation

**Not in `main`.** The engine runs single-process today. The multi-process work (network
process, decoder process, per-navigation renderer) lives on `origin/process-isolation-docs` and
the `upstream/stack/*` branches and has not merged. There is no `docs/process-isolation.md` on
`main`, and the sandboxing, wire protocol and crash isolation described on those branches are
not in this tree.


## JavaScript

**Not wired.** No page executes script. Five crates exist and build - `gosub_webexecutor` (the
runtime abstraction), `gosub_v8` (V8 bindings), `gosub_webinterop` (proc-macro glue),
`gosub_jsapi` (console, fetch headers, URL, storage, events, text encoding, base64,
DOMException), and `gosub_web_platform` (event loop, timers, listeners) - but `gosub_engine`
depends on none of them, and `TabCommand::ExecuteScript` is declared and not handled.

Script runs in one place: the WPT harness, which uses QuickJS rather than V8. `gosub_domjs`
binds the DOM to QuickJS through `rquickjs` so that web-platform-tests can drive the engine's
document (`document`, `node`, `text`, `event`, `style`, `select`, `timers`). It is test-only and
holds no DOM logic of its own.


## Embedder API - `gosub_engine`

**Works.** `GosubEngine` is generic over a configuration type naming every swappable component,
with `DefaultRenderConfig<Backend, FontSystem, Compositor>` as the ready-made choice. Underneath
it: zones as isolated profiles with their own cookies and storage, tabs as independent async
worker tasks, and an event bus. 35 `EngineEvent` variants flow out and 34 `TabCommand` variants
flow in. Navigation, history, hit testing, mouse and keyboard input, text editing, downloads,
clipboard, cursor and focus changes, and the picker protocol for `<input type=color|date|time>`
all work end to end - the embedder opens the platform's own picker and answers with
`PickerChanged`/`PickerClosed`.

**Not yet.** Nine of the 34 `TabCommand` variants are declared and never handled; they reach the
worker's catch-all and log `received unhandled command`. They are `SetCookie`, `ClearCookies`,
`SetStorageItem`, `RemoveStorageItem`, `ClearStorage`, `ExecuteScript`, `PlayMedia`,
`PauseMedia` and `DumpDomTree`. Cookies and storage are reachable through the zone services
instead, so only the per-tab shortcut is missing. Script and media have nothing behind them.
Regenerate the list by diffing the enum against the arms in `engine/tab/worker.rs`.

Form controls render and accept input for `text`, `password`, `checkbox`, `radio`, `button`,
`submit`, `file`, `hidden`, `range` and `color`.


## Cookies and storage

**Works.** A cookie jar with a persistent SQLite-backed store and per-zone isolation, with
partitioning policy configurable per zone. localStorage and sessionStorage both have in-memory
implementations; localStorage also has a SQLite store. Storage changes raise `StorageChanged`.


## Conformance

The numbers live in [wpt.md](wpt.md) and are regenerated with `make wpt-update`, so they are not
repeated here. The engine passes a low single-digit percentage of the testharness corpus and
around a quarter of the CSS2 reftests. The reftest rate is higher because reftests exercise
layout and painting, which the engine does, while testharness suites exercise DOM and Web APIs,
which mostly need the scripting that is not wired up.

That sits next to 99.88% on html5lib. The difference is the scripting and the Web APIs, not the
parsing.
