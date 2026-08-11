# Examples

This directory contains runnable examples for the Gosub engine. They are split into two groups:
engine examples (headless, no GUI required) and GUI examples (need system graphics libraries).


## Engine examples

These use the `NullBackend` — no window, no GPU, no extra system packages needed. Start here.

### `hello-world`

A single tab that navigates a URL and streams all engine events to stdout. Shows the full
lifecycle: engine setup → zone → tab → navigate → event loop → shutdown.

```bash
cargo run --example hello-world
```

### `multi-tab`

25 tabs navigating random sites simultaneously. Uses `indicatif` progress bars to show live
per-tab status (navigation state, resource progress). Good for observing the async networking
stack under load.

```bash
cargo run --example multi-tab
```

### `html5-parser`

Parses an HTML document using `gosub_html5` directly (bypasses `GosubEngine`) and prints the
resulting DOM tree. Useful for working on the parser in isolation.

```bash
cargo run --example html5-parser
```

### `tutorial`

The companion to [`docs/tutorial.md`](../docs/tutorial.md): the minimal
Engine → Zone → Tab → Navigate → Event loop → Shutdown lifecycle.

```bash
cargo run --example tutorial -- https://example.com
```

### `pipeline-test`

End-to-end smoke test: serves a known page from a tiny local HTTP server, navigates the engine
to it, and asserts that navigation completes and all sub-resources (CSS/JS/image) are fetched.

```bash
cargo run --example pipeline-test
```

### `config-store`

View and modify the configuration store (`list`, `search`, `set`, ...).

```bash
cargo run --example config-store list
```

### `metrics-cli`

Fetches and displays timing stats from a running engine's metrics endpoint; supports `--watch`,
`--json` and `--reset`.

```bash
cargo run --example metrics-cli
```


## GUI examples

These open a real window. Each is its own package with a single binary, so they run with
`cargo run -p example-<name>`, and all accept a URL as the first argument. They require system
graphics libraries — see the [installation instructions](../README.md#running-the-examples) in
the root README. The full toolkit × backend matrix (winit / GTK4 / egui × Cairo / Skia /
Skia-GPU / Vello) is documented in [`docs/examples.md`](../docs/examples.md):

```bash
cargo run -p example-winit-vello    # winit window, Vello/wgpu GPU rendering
cargo run -p example-winit-skia     # winit window, Skia CPU rendering
cargo run -p example-winit-skia-gpu # winit window, Skia GPU (OpenGL) rendering
cargo run -p example-winit-cairo    # winit window, Cairo CPU rendering
cargo run -p example-gtk4-cairo     # GTK4 window, Cairo CPU rendering (Pango text)
cargo run -p example-gtk4-skia      # GTK4 window, Skia CPU rendering
cargo run -p example-gtk4-skia-gpu  # GTK4 window, Skia GPU (OpenGL/GLArea) rendering
cargo run -p example-egui-vello     # egui window, Vello/wgpu GPU rendering
cargo run -p example-egui-skia      # egui window, Skia CPU rendering
cargo run -p example-egui-cairo     # egui window, Cairo CPU rendering
```


## Choosing a configuration

`GosubEngine` is generic over a *configuration* type that names its pluggable components at
compile time. You normally use `DefaultRenderConfig<Backend, FontSystem>` — the ready-made
config that wires the standard gosub HTML/CSS stack and lets you pick the backend and font
system. Every GUI example defines a one-line alias for its choice and reuses it everywhere:

```rust
// from winit-cairo
type AppConfig = DefaultRenderConfig<CairoBackend, PangoFontSystem>;
// ...then: GosubEngine::<AppConfig>::new(...)
```

The headless examples use the parameter-less `DefaultRenderConfig`, which defaults to
`DefaultRenderConfig<NullBackend, ParleyFontSystem, DefaultCompositor>`. See the
"Configuration" section of the `gosub_engine` crate docs for the full picture (including the
`ModuleConfiguration` / `RenderConfiguration` trait split for parse-only vs. rendering setups).

## Writing your own example

The shortest useful starting point is `hello-world.rs`. The pattern is:

1. Pick a backend + font system and alias it: `type AppConfig = DefaultRenderConfig<Backend, FontSystem>;`
   (or just `DefaultRenderConfig` with no params for headless `NullBackend`)
2. Create `GosubEngine::<AppConfig>` with that backend
3. Call `engine.start()`
4. Create a zone → create a tab → send `TabCommand::Navigate`
5. Drive events in a `tokio::select!` loop
6. Call `engine.shutdown().await` before exiting
