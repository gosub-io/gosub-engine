# Running the examples

The repository ships runnable examples in two groups: headless engine examples (no GUI), and GUI
examples that open a real window. For a narrative description of what each example does, see
[`examples/README.md`](../examples/README.md). For how the engine is configured per backend, see
[`configuration.md`](configuration.md).

## Installing dependencies

This project uses [cargo](https://doc.rust-lang.org/cargo/) and
[rustup](https://www.rust-lang.org/tools/install). Install `rustup`, then:

```bash
rustup default stable
git clone https://github.com/gosub-io/gosub-engine.git
cd gosub-engine
cargo build
```

**OS packages required for GTK4 and Cairo examples** (Ubuntu / Debian):

```
make gcc g++
libglib2.0-dev libcairo2-dev libpango1.0-dev
libgdk-pixbuf-2.0-dev libgraphene-1.0-dev libgtk-4-dev
libsqlite3-dev
```

The winit-vello, egui-vello, and gosub-screenshot binaries have no system-library dependencies and
build out of the box on Linux, macOS, and Windows.

## Engine examples (no GUI required)

| Command | Description |
|---|---|
| `cargo run --example hello-world` | Single tab — navigate a URL, stream events to stdout |
| `cargo run --example multi-tab` | 25 tabs navigating random sites; live progress bars via `indicatif` |
| `cargo run --example tutorial -- <url>` | The companion to [`tutorial.md`](tutorial.md) |

## GUI examples

All GUI examples accept a URL as the first argument, e.g. `-- https://example.com`.

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
| `cargo run -p gosub-screenshot -- <url> [out.png]` | Render a URL to a full-page PNG without opening a window (CPU Skia, statically linked — no GPU or system libraries) |

See [headless.md](headless.md) for how the tool drives the engine and how to build your own
headless integration.

## Component tools (individual crate testing)

| Command | Description |
|---|---|
| `cargo run --bin gosub-parser` | HTML5 parser / tokenizer — prints a document tree |
| `cargo run --bin css3-parser` | CSS3 parser — prints a CSS tree from a URL |
| `cargo run --bin display-text-tree` | Text-only render of a page |
| `cargo run --bin config-store` | Config store smoke test |
| `cargo run --bin run-js` | Run a JS file (event loop not yet implemented) |
| `cargo run --bin html5-parser-test` | html5lib tree-builder test suite |
| `cargo run --bin parser-test` | Parser development test runner |

For more detail on the component tools see [`binaries.md`](binaries.md).
