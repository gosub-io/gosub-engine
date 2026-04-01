# Gosub Engine Architecture

## Overview

Gosub Engine is an **embeddable, modular browser rendering engine** written in Rust. It is structured as a **24-crate workspace** where every major component (HTML parsing, CSS, layout, rendering) is swappable via a trait-based abstraction layer. The system is designed to be embedded inside a host application (e.g., a GTK window) and exposes an event-driven async API.

---

## Workspace Structure

```
crates/
├── gosub_interface        Core trait definitions (the contract)
├── gosub_stream           Byte stream & encoding detection
├── gosub_config           Configuration management
├── gosub_html5            HTML5 parser & DOM
├── gosub_css3             CSS3 parser & cascade
├── gosub_net              Networking & HTTP
├── gosub_storage          Cookie & session storage
├── gosub_fontmanager      Font management & measurement
├── gosub_taffy            Taffy layout engine wrapper
├── gosub_rendering        Render tree construction
├── gosub_renderer         Low-level rendering utilities
├── gosub_pipeline         Full rendering pipeline
├── gosub_cairo            Cairo graphics backend
├── gosub_vello            Vello GPU graphics backend
├── gosub_svg              SVG rendering
├── gosub_webexecutor      JavaScript execution interface
├── gosub_v8               V8 JavaScript engine bindings
├── gosub_jsapi            Standard Web JavaScript APIs
├── gosub_webinterop       Rust↔JS interop proc macros
├── gosub_web_platform     Event loop & async web platform
├── gosub_instance         Instance lifecycle management
├── gosub_engine_core      Main engine orchestration
└── gosub_tools            CLI tools & renderer examples
```

---

## Layer Model

The crates form a layered dependency graph from bottom (abstractions) to top (orchestration):

```
┌─────────────────────────────────────────────────────────────┐
│  gosub_engine_core  (orchestration, zone/tab management)    │
├──────────────┬──────────────────┬──────────────────────────┤
│ gosub_pipeline (rendering pipeline: layout → paint → raster)│
├──────┬───────┴──┬───────────────┼──────────────────────────┤
│cairo │  vello   │  taffy        │  rendering   │  renderer  │
├──────┴──────────┴───────────────┴──────────────┴───────────┤
│         gosub_html5    gosub_css3    gosub_net              │
├────────────────────────────────────────────────────────────┤
│       gosub_stream     gosub_config    gosub_fontmanager    │
├────────────────────────────────────────────────────────────┤
│                  gosub_interface  (traits)                  │
└────────────────────────────────────────────────────────────┘
```

---

## The Trait System (gosub_interface)

The entire architecture is parameterized by a single **`ModuleConfiguration`** trait that ties all component implementations together. This is the key design pattern: every crate depends on abstract traits, not concrete types.

```rust
trait ModuleConfiguration:
    HasDocument +
    HasCssSystem +
    HasLayouter +
    HasRenderTree +
    HasRenderBackend +
    HasFontManager +
    ...
```

Each sub-trait defines an associated type for one component:

| Trait              | Associated Type     | Default Implementation     |
|--------------------|---------------------|----------------------------|
| `HasDocument`      | `Document`          | `gosub_html5::DocumentImpl` |
| `HasCssSystem`     | `CssSystem`         | `gosub_css3::Css3System`    |
| `HasLayouter`      | `Layouter`          | `gosub_taffy::TaffyLayouter`|
| `HasRenderTree`    | `RenderTree`        | `gosub_rendering::RenderTree`|
| `HasRenderBackend` | `RenderBackend`     | Cairo / Vello / Skia        |
| `HasFontManager`   | `FontManager`       | `gosub_fontmanager`         |

A host application wires this together by implementing a concrete `Config` struct:

```rust
struct Config;
impl HasCssSystem for Config  { type CssSystem = Css3System; }
impl HasDocument for Config   { type Document = DocumentImpl<Self>; ... }
impl HasLayouter for Config   { type Layouter = TaffyLayouter; ... }
impl HasRenderBackend for Config { type RenderBackend = CairoBackend; }
```

This gives compile-time guarantees that all parts fit together, while allowing any component to be replaced.

---

## Data Flow: HTML In → Rendered Output

The processing pipeline has eight distinct stages:

### 1. Fetching
```
URL ──► gosub_net::fetch() ──► HTTP response ──► raw bytes
```
- Non-WASM: uses `reqwest`
- WASM: uses browser `fetch` via `web-sys`
- Cookies managed per-zone by `gosub_storage`

### 2. Stream & Encoding
```
raw bytes ──► gosub_stream::ByteStream (encoding detection via chardetng)
          ──► normalized UTF-8 stream
```

### 3. HTML Parsing
```
ByteStream ──► Tokenizer ──► HTML tokens
           ──► Parser    ──► DOM tree (Document<C>)
```
The HTML5 parser in `gosub_html5` produces a DOM tree that implements the `Document<C>` trait. Inline `<style>` tags are forwarded to the CSS parser as they are encountered.

### 4. CSS Parsing & Cascade
```
<link> / <style> ──► gosub_css3::Css3::parse_str() ──► Stylesheet AST
DOM + Stylesheets ──► CssSystem::compute_cascade()  ──► CssPropertyMap (per node)
```
The UA stylesheet is loaded once and merged with author styles. Specificity, inheritance, and the cascade are handled by `gosub_css3`.

### 5. Render Tree Construction
```
DOM + CssPropertyMap ──► gosub_rendering::RenderTree::build()
                     ──► RenderTree<C>  (nodes with computed styles)
```
Nodes with `display: none` are excluded. The render tree is a separate structure from the DOM.

### 6. Layout
```
RenderTree ──► gosub_taffy::TaffyLayouter::layout()
           ──► layout information (position + size) attached to each node
```
Taffy implements Flexbox, Grid, and block flow. Text measurement is delegated to Parley or a simpler backend.

### 7. Paint & Rasterize (gosub_pipeline)
```
RenderTree
    │
    ▼
[bridge] ──► PipelineDocument (pipeline-internal representation)
    │
    ▼
[rendertree_builder] ──► intermediate render tree
    │
    ▼
[layouter] ──► computed positions (via Taffy)
    │
    ▼
[layering] ──► LayerList  (z-index, stacking contexts)
    │
    ▼
[tiler]    ──► TileList   (256×256 tiles for large pages)
    │
    ▼
[painter]  ──► paint operations (rects, text, images, SVG)
    │
    ▼
[rasterizer] ──► backend-specific pixel data
    │
    ▼
[compositor] ──► final composite image ──► screen / buffer
```

### 8. Display
The composited result is handed to the host application. For GTK4 it is drawn onto a Cairo surface; for Vello it is presented via wgpu.

---

## Rendering Backends

The rendering backend is swappable via `HasRenderBackend`. Three backends exist:

| Backend        | Crate           | Technology             | Use Case                        |
|----------------|-----------------|------------------------|---------------------------------|
| Cairo          | `gosub_cairo`   | Cairo + Pango + GTK4   | Desktop Linux/GTK integration   |
| Vello          | `gosub_vello`   | Vello + wgpu (GPU)     | Modern GPU-accelerated rendering|
| Skia           | `gosub_pipeline`| Skia (optional)        | Advanced GPU + text layout      |
| Null           | built-in        | No output              | Testing & headless              |

All backends implement the `RenderBackend` trait:
```rust
trait RenderBackend {
    fn draw_rect(&mut self, ...);
    fn draw_text(&mut self, ...);
    fn apply_scene(&mut self, ...);
    fn activate_window(&mut self, ...);
    fn render(&mut self, ...);
}
```

---

## Engine Core Architecture (gosub_engine_core)

### Components

**`GosubEngine`** — Central orchestrator
- Manages zones and their tabs
- Owns the broadcast event channel
- Spawns an I/O thread for networking
- Holds the render backend

**`Zone`** — Profile / session container
- Isolated cookie jar and storage
- Can contain multiple tabs
- Models a browser profile

**`Tab` / `TabWorker`** — Browsing context
- One page being parsed and rendered
- Receives `TabCommand` (Navigate, SetViewport, SendInput, …)
- Emits `EngineEvent` (Navigation, Resource, Redraw, Error, …)

### Event System

```
Host application
      │
      │  TabCommand (MPSC)
      ▼
  TabWorker
      │
      │  EngineEvent (broadcast)
      ▼
All subscribers (UI, DevTools, tests, …)
```

Key event types:

| Event                        | Meaning                                |
|------------------------------|----------------------------------------|
| `EngineEvent::Navigation`    | Page load state changed                |
| `EngineEvent::Resource`      | A sub-resource is loading/done         |
| `EngineEvent::Redraw`        | A new rendered frame is available      |
| `EngineEvent::Error`         | An error occurred in the engine        |

---

## JavaScript Architecture

JavaScript support is layered as follows:

```
gosub_webinterop   (proc macros: #[js_bind], generates FFI glue)
       │
gosub_webexecutor  (abstract JS execution trait)
       │
gosub_v8           (concrete V8 engine implementation)
       │
gosub_jsapi        (standard Web APIs: setTimeout, fetch, …)
       │
gosub_web_platform (async event loop, worker threads, Tokio runtime)
```

The `gosub_webinterop` proc-macro crate generates the binding code to expose Rust structs to JavaScript and vice versa.

---

## Concurrency Model

The engine is built on **Tokio**:

- `GosubEngine` runs as an async Tokio task
- Each `Zone` and `Tab` runs in a separate task
- A dedicated I/O thread handles network requests
- Commands flow via MPSC channels (single producer → worker)
- Events flow via broadcast channels (worker → many subscribers)
- All public types implement `Send + Sync`

---

## Configuration & Storage

**`gosub_config`** stores persistent browser settings. On non-WASM targets it uses SQLite via `rusqlite`. On WASM it falls back to in-memory storage.

**`gosub_storage`** provides trait-based storage backends:
- `InMemoryLocalStore` / `InMemorySessionStore` for testing
- SQLite-backed store (via `r2d2_sqlite`) for production

---

## Key Design Patterns

1. **Single Config type parameter** — ties all component implementations together at compile time; no runtime dispatch overhead for the core pipeline
2. **Trait-based pluggability** — every major component (parser, CSS, layout, renderer) can be replaced by implementing the relevant trait
3. **Handle-based remote control** — `TabHandle` / `ZoneHandle` allow controlling tabs from outside their task
4. **Event-driven** — broadcast channels decouple the engine from the host application
5. **Async-first** — Tokio throughout; blocking I/O is isolated to a dedicated thread
6. **Pipeline stages** — the rendering pipeline (`gosub_pipeline`) is a clean sequence of independent transformations
7. **Multi-zone isolation** — cookie jars and storage are per-zone, modeling browser profiles
