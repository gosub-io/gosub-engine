# Running the examples

The repository ships runnable examples in two groups: headless engine examples (no GUI), and GUI examples that open a real window. For a narrative description of what each example does, see [`examples/README.md`](../examples/README.md). For how the engine is configured per backend, see [`configuration.md`](configuration.md).

## Installing dependencies

This project uses [cargo](https://doc.rust-lang.org/cargo/) and [rustup](https://www.rust-lang.org/tools/install). Install `rustup`, then:

``` bash
rustup default stable
git clone https://github.com/gosub-io/gosub-engine.git
cd gosub-engine
cargo build
```

**OS packages required for the GTK4 and Cairo examples** (Ubuntu / Debian):

    make gcc g++
    libglib2.0-dev libcairo2-dev libpango1.0-dev
    libgdk-pixbuf-2.0-dev libgraphene-1.0-dev libgtk-4-dev
    libsqlite3-dev

**OS packages required for the Skia binaries** — `gosub-screenshot` and the `*-skia` examples
(Ubuntu / Debian):

    libfontconfig-dev libfreetype-dev

Skia links fontconfig and freetype itself, so these are needed even by `gosub-screenshot`,
which opens no window.

Everything else builds with no system packages at all: the engine examples below, `gosub-wpt`,
and the winit-vello and egui-vello binaries, which build out of the box on Linux, macOS and
Windows.

## Engine examples (no GUI required)

| Command | Description |
|---|---|
| `cargo run --example hello-world` | Single tab — navigate a URL, stream events to stdout |
| `cargo run --example multi-tab` | 25 tabs navigating random sites; live progress bars via `indicatif` |
| `cargo run --example tutorial -- <url>` | The companion to [`tutorial.md`](tutorial.md) |
| `cargo run --example html5-parser` | Parse a document with `gosub_html5` alone and print the DOM tree |
| `cargo run --example pipeline-test` | End-to-end smoke test against a local HTTP server |
| `cargo run --example metrics-cli` | Timing stats from a running engine; `--watch`, `--json`, `--reset` |

## GUI examples

The GUI examples are intentionally minimal embedders: load the URL from the command line,
scroll, hover, click links (the GTK/egui ones keep a toolkit-native address bar; the winit ones
have no chrome at all). For the full interactive feature set (typing into forms, clipboard,
cursor shapes, kinetic scrolling) use `cargo run --release -p gosub-mini-browser` — see
[binaries.md](binaries.md).

All GUI examples accept a URL as the first argument, e.g. `-- https://example.com`.

### winit (cross-platform, no GTK required)

| Command | Renderer | Notes |
|---|---|---|
| `cargo run -p example-winit-vello` | Vello / wgpu | Cross-platform — Metal, DX12, Vulkan |
| `cargo run -p example-winit-skia` | Skia CPU | softbuffer presentation |
| `cargo run -p example-winit-skia-gpu` | Skia GPU (OpenGL) | OpenGL compositing |
| `cargo run -p example-winit-cairo` | Cairo CPU | Linux; needs libcairo |

### GTK4 (Linux, requires GTK4 system packages)

| Command | Renderer | Notes |
|---|---|---|
| `cargo run -p example-gtk4-cairo` | Cairo CPU | Pango text rendering |
| `cargo run -p example-gtk4-skia` | Skia CPU | |
| `cargo run -p example-gtk4-skia-gpu` | Skia GPU (OpenGL/GLArea) | Hardware-accelerated compositing |

### egui

| Command | Renderer | Notes |
|---|---|---|
| `cargo run -p example-egui-vello` | Vello / wgpu | Cross-platform |
| `cargo run -p example-egui-skia` | Skia CPU | |
| `cargo run -p example-egui-cairo` | Cairo CPU | Linux; needs libcairo |

## Headless tool

| Command | Description |
|---|---|
| `cargo run -p gosub-screenshot -- <url> [out.png]` | Render a URL to a full-page PNG without opening a window — CPU Skia, no GPU and no window system (it does link fontconfig and freetype) |

See [headless.md](headless.md) for how the tool drives the engine and how to build your own headless integration.

## Component tools (individual crate testing)

| Command | Description |
|---|---|
| `cargo run --bin gosub-parser` | HTML5 parser / tokenizer — prints a document tree |
| `cargo run --bin css3-parser` | CSS3 parser — prints a CSS tree from a URL |
| `cargo run --bin css-check` | Parse a CSS file/URL, warn on every unparsable rule |
| `cargo run --bin display-text-tree` | Text-only render of a page |
| `cargo run --example config-store` | Config store smoke test (example target, not a bin) |
| `cargo run --bin run-js` | Run a JS file (event loop not yet implemented) |
| `cargo run --bin html5-parser-test` | html5lib tree-builder test suite |
| `cargo run --bin parser-test` | Parser development test runner |
| `cargo run -p gosub_lattice --bin table_console` | Table layout engine console demos |
| `cargo run -p generate_definitions` | Regenerate the gosub_css3 CSS definition JSON (`-- --property-ids` regenerates the property-id module offline) |
| `cargo run -p gosub_render_pipeline --example style_dump -- <out-dir>` | Dump every element's computed style for a set of page fixtures, so a change to the style system can be diffed against itself |
| `cargo run -p gosub_render_pipeline --example render_dump -- <out-dir>` | Dump the laid-out boxes and paint commands of the same fixtures, so a change can be proved to move no pixel |

For more detail on the component tools see [`binaries.md`](binaries.md).

### style_dump

`style_dump` exists for changes to the style system. It writes down, for every element of a set
of page fixtures, every declaration that reached each property with its cascade facts, and the
cascaded, specified, computed, used, actual and inherited value the property settled on. Run it
before a change and after; the two directories have to be identical unless the change was meant
to alter what pages compute to.

```bash
cargo run -p gosub_render_pipeline --example style_dump -- /tmp/style/before
# ... make the change ...
cargo run -p gosub_render_pipeline --example style_dump -- /tmp/style/after
diff -r /tmp/style/before /tmp/style/after
```

The fixtures are the ones the `style` benchmark measures plus the page fixtures in `tests/data`,
and the pages themselves are shared with that benchmark, so the two describe one thing. Keys are
property names and every list is sorted, so the output stays comparable across a change to how
the engine keys itself internally.

### render_dump

`style_dump` proves the cascade decided the same thing; `render_dump` proves the pipeline made
the same page of it. For each of the same fixtures it runs stages 1-5 and writes
`<name>.layout.json` (tag, id, class, depth and border box of every element in document order,
the layouter's own `GOSUB_DUMP_LAYOUT` output) and `<name>.paint.txt` (the paint commands of
every tile of every layer, in layer and tile order).

```bash
cargo run --release -p gosub_render_pipeline --example render_dump -- /tmp/render/before
# ... make the change ...
cargo run --release -p gosub_render_pipeline --example render_dump -- /tmp/render/after
diff -rq /tmp/render/before /tmp/render/after
```

One caveat: the two `stackoverflow` fixtures are not reproducible run to run. Their page has an
`<img>` whose media cannot be fetched, and whether the second request for it finds the
placeholder already installed depends on how fast the first fetch fails - so the image is
sometimes 0x0 and sometimes the placeholder's 32x32, and the page below it shifts by 6px. The
other 28 fixtures are stable.

## Benchmarks

Two benchmarks in the render pipeline crate, both Criterion, both gating a different half of the
work a page costs. They share their fixtures with the dump tools above, so a number and a dump
describe the same page.

```bash
cargo bench -p gosub_render_pipeline --bench style -- --save-baseline before
cargo bench -p gosub_render_pipeline --bench scroll -- --save-baseline before
# ... make the change ...
cargo bench -p gosub_render_pipeline --bench style -- --baseline before
cargo bench -p gosub_render_pipeline --bench scroll -- --baseline before
```

`style` measures giving every element its computed style: `cascade` is the CSS crate alone,
`render-tree` is stage 1 of the pipeline over it.

`scroll` measures what a laid-out page costs to keep painted while it scrolls, which re-runs
neither styling nor layout: `first-window` builds the tile grid and paints the first raster
window, `scroll-through` walks the page in 300px steps, each step doing what the engine's extend
path does - reuse the grid, keep what earlier passes painted, park what is outside the raster
window, paint the rest.

Two things to know before reading a `scroll` number. It runs on the layouter's own Parley font
system, because the render pipeline has no backend of its own, while the browser shares the
rasterizer's - which on the Cairo path is Pango, and far dearer. A scroll step measures around
0.6 ms here against roughly 39 ms through the Cairo screenshot tool, so this is a regression gate
for tiling, painting and cache logic rather than a model of absolute scroll cost. For the latter,
use `gosub-screenshot --viewport-height 800 --timings` with replayed `-i scroll:0,300`
interactions. And the fixtures do not lay out identically on every platform, since the layouter
measures with whatever system fonts the machine has, so a baseline is only comparable on the
machine that recorded it.

Baselines are per target directory as well as per machine, so a tree that shares
`CARGO_TARGET_DIR` with another shares its baselines too. Take numbers that decide anything on a
quiet, dedicated machine, back to back, with a target directory per source tree.
