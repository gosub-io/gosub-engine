# Render Pipeline

The Gosub render pipeline transforms a parsed HTML document into pixel data displayed in a window. It is a **staged, tile-based pipeline** that runs entirely on the CPU by default, with optional GPU acceleration via the Vello backend.

The pipeline lives behind the `pipeline` Cargo feature. When that feature is disabled, the engine falls back to a placeholder gray clear (only when no document is loaded) — no layout or painting occurs.

## Quick navigation

| Document | Contents |
|----------|----------|
| [Stages](stages.md) | Deep dive into each of the 7 pipeline stages |
| [Data structures](data-structures.md) | Key types and how they flow between stages |
| [Backends](backends.md) | How render backends consume the display list |

## Architecture overview

```
  EngineDocument (DOM + stylesheets)
         │
         ▼
  ┌──────────────────────────────────┐
  │  Stage 1 – Render Tree Builder   │  crates/gosub_pipeline/src/rendertree_builder/
  │  Filter invisible nodes          │
  └──────────────┬───────────────────┘
                 │  RenderTree
                 ▼
  ┌──────────────────────────────────┐
  │  Stage 2 – Layout (Taffy)        │  crates/gosub_pipeline/src/layouter/
  │  Compute box models              │
  └──────────────┬───────────────────┘
                 │  LayoutTree
                 ▼
  ┌──────────────────────────────────┐
  │  Stage 3 – Layering              │  crates/gosub_pipeline/src/layering/
  │  Assign elements to layers       │
  └──────────────┬───────────────────┘
                 │  LayerList
                 ▼
  ┌──────────────────────────────────┐
  │  Stage 4 – Tiling                │  crates/gosub_pipeline/src/tiler.rs
  │  Subdivide viewport into tiles   │
  └──────────────┬───────────────────┘
                 │  TileList
                 ▼
  ┌──────────────────────────────────┐
  │  Stage 5 – Painting              │  crates/gosub_pipeline/src/painter/
  │  Generate paint commands         │
  └──────────────┬───────────────────┘
                 │  TileList (with PaintCommands)
                 ▼
  ┌──────────────────────────────────┐
  │  Stage 6 – Rasterization         │  crates/gosub_pipeline/src/rasterizer/
  │  Execute commands → pixel data   │
  └──────────────┬───────────────────┘
                 │  TextureStore
                 ▼
  ┌──────────────────────────────────┐
  │  Stage 7 – Compositing           │  context.rs (pipeline_render)
  │  Assemble tiles → display list   │
  └──────────────┬───────────────────┘
                 │  RenderList (DisplayItem::Blit …)
                 ▼
  ┌──────────────────────────────────┐
  │  Render Backend                  │  crates/gosub_cairo / gosub_vello
  │  Draw display list to window     │
  └──────────────────────────────────┘
```

## Entry points

### Triggering a rebuild

`BrowsingContext` (in `crates/gosub_engine/src/engine/context.rs`) owns the dirty flags and the cached `RenderList`. Three methods mark the pipeline as needing a rebuild:

```rust
context.set_document(doc);     // new HTML loaded
context.set_viewport(vp);      // window resized
context.invalidate_render();   // DOM/CSS changed
```

### Running the pipeline

```rust
// Called each draw tick; no-ops when clean
context.rebuild_render_list_if_needed();
```

Internally this calls the free function `pipeline_render(doc, viewport, &mut render_list)` which runs all seven stages and populates the `RenderList`.

## Feature flags

| Flag | Enables | Requires |
|------|---------|----------|
| `pipeline` | The pipeline crate and all 7 stages | — |
| `backend_cairo` | Cairo rasterizer + GTK4 window | `pipeline`, `text_pango`, GTK4 |
| `backend_vello` | Vello GPU rasterizer | `pipeline`, `wgpu`, `winit` |
| `text_pango` | Pangocairo text layout | GTK4 |
| `text_parley` | Pure-Rust Parley text layout | — |
| `text_skia` | Skia text layout | skia-safe |

A typical development build with Cairo:

```
cargo build --features backend_cairo
```

When neither `backend_cairo` nor `backend_vello` is enabled, `DisplayItem::Blit` commands are emitted into the render list but no backend is wired to consume them, so nothing is visible. This is intentional — new backends can be developed against the same display list.

## Key constants

| Name | Value | Where | Purpose |
|------|-------|-------|---------|
| Tile size | 256 × 256 px | `context.rs` | Default tile dimensions |
| `DEFAULT_FONT_SIZE` | 16.0 px | `layouter/taffy.rs` | Fallback font size |
| `DEFAULT_FONT_FAMILY` | `"Sans"` | `layouter/taffy.rs` | Fallback font family |
| Invisible tags | `head style script meta link title` | `rendertree_builder/tree.rs` | Nodes pruned from render tree |
