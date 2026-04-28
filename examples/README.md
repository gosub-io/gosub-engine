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


## GUI examples

These open a real window. They require system graphics libraries — see the
[installation instructions](../README.md#running-the-examples) in the root README.

### `gtk-cairo`

A GTK4 window backed by Cairo. The render backend is `CairoBackend` from `gosub_cairo`.

```bash
cargo run --example gtk-cairo
```

### `egui-vello`

An egui window backed by Vello and wgpu. The render backend is from `gosub_vello`.

```bash
cargo run --example egui-vello
```

### `gtk-renderer` / `vello-renderer`

Earlier renderer prototypes that predate `GosubEngine`. They drive the HTML/CSS/layout pipeline
directly without going through the unified engine entry point. Kept for reference while the new
rendering path matures.

```bash
cargo run --example gtk-renderer
cargo run --example vello-renderer
```


## Writing your own example

The shortest useful starting point is `hello-world.rs`. The pattern is:

1. Pick a backend (`NullBackend` for headless, `CairoBackend` / Vello for GUI)
2. Create `GosubEngine` with that backend
3. Call `engine.start()`
4. Create a zone → create a tab → send `TabCommand::Navigate`
5. Drive events in a `tokio::select!` loop
6. Call `engine.shutdown().await` before exiting
