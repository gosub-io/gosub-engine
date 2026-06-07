# Render Pipeline

The Gosub render pipeline transforms a parsed HTML document into pixel data displayed in a window. It is a **staged, tile-based pipeline** that caches rasterized tiles across frames and only re-runs expensive stages when content actually changes.

The pipeline lives behind the `pipeline` Cargo feature. When that feature is disabled the engine falls back to a placeholder grey clear — no layout or painting occurs.

## Quick navigation

| Document | Contents |
|---|---|
| [Stages](stages.md) | Deep dive into each of the 7 pipeline stages |
| [Data structures](data-structures.md) | Key types and how they flow between stages |
| [Backends](backends.md) | Render backends, ExternalHandle, and host compositing |

## Two-phase design

The pipeline is split into two phases with very different costs:

### Phase 1 — Build cache (stages 1–6)

Runs inside `pipeline_build_cache()` in `crates/gosub_engine/src/engine/context.rs`.  
Triggered by: navigation, DOM/CSS change, viewport resize.  
Typical cost: **50–500 ms** depending on page complexity.

```
  DOM + stylesheets
       │
       ▼  Stage 1 – Render Tree Builder       filter invisible nodes
       │  RenderTree
       ▼  Stage 2 – Layout (Taffy)            compute box models
       │  LayoutTree
       ▼  Stage 3 – Layering                  assign elements to layers
       │  LayerList
       ▼  Stage 4 – Tiling                    subdivide page into 256×256 tiles
       │  TileList
       ▼  Stage 5 – Painting                  generate backend-agnostic PaintCommands
       │  TileList (with PaintCommands)
       ▼  Stage 6 – Rasterization             execute paint commands → pixel buffers
       │
       └─► PipelineCache  { BakedTile[], CachedTile[], LayerList, page_height }
```

`BakedTile` and `CachedTile` both hold `Arc<Vec<u8>>` pixel data so tiles can be shared across frames with zero copies.

### Phase 2 — Composite (stage 7)

Runs every frame, including pure scroll events. Typical cost: **< 1 ms**.

For the **Cairo** and **display-list** paths, stage 7 selects the visible tiles from the cache, applies the scroll offset, and emits `DisplayItem::Blit` entries into a `RenderList`. The `RenderBackend::render()` call then processes that list.

For the **Skia** path, stage 7 is bypassed entirely. The engine submits an `ExternalHandle::TileCache` directly to the compositor. The host window thread receives the tile list and composites it — no backend render call required.

## Architecture diagram

```
   DOM + CSS
      │
      │  pipeline_build_cache()        ← only on content change
      │
      ├─ Stage 1  RenderTree
      ├─ Stage 2  LayoutTree
      ├─ Stage 3  LayerList
      ├─ Stage 4  TileList
      ├─ Stage 5  TileList + PaintCommands
      ├─ Stage 6  TextureStore  ←────────── CairoRasterizer / SkiaRasterizer
      │
      └─► PipelineCache (BakedTile[], CachedTile[])
               │
               │  rebuild_render_list_if_needed() / tile_cache_handle()
               │                                  ← every frame
               │
        ┌──────┴────────────────────────────────────────────┐
        │                                                   │
        │  Cairo / display-list path       Skia path        │
        │                                                   │
        ▼                                                   ▼
  pipeline_composite()                ExternalHandle::TileCache
  → RenderList (Blit items)           (Arc<Vec<u8>> per tile,
        │                              zero-copy, scroll embedded)
        ▼
  RenderBackend::render()
  → ExternalHandle (CpuPixelsOwned / …)
        │
        └──────────────────────────────┘
                       │
                       ▼
         DefaultCompositor::submit_frame()
                       │
                       ▼
           Host window thread redraws
        (GTK draw / winit redraw / egui)
                       │
                       ▼
               Pixels on screen
```

## Entry points

### Triggering a rebuild

`BrowsingContext` in `crates/gosub_engine/src/engine/context.rs` owns the dirty flags:

```rust
context.set_document(doc);       // navigation — full rebuild
context.set_viewport(vp);        // resize — full rebuild
context.invalidate_render();     // DOM/CSS changed — full rebuild
context.set_scroll(x, y);        // scroll only — stage 7, no rasterization
```

### Consuming the pipeline output

```rust
// Rebuild cache if dirty, then populate render list (Cairo / display-list path).
context.rebuild_render_list_if_needed();

// Zero-copy scroll handle (Cairo fast path): Some only when scroll_dirty && !render_dirty.
context.take_scroll_handle(dpr) -> Option<ExternalHandle::TileCache>

// Tile cache handle (Skia path): always returns Some if cache is populated.
context.tile_cache_handle(dpr)  -> Option<ExternalHandle::TileCache>
```

### tick_draw fast-path order

`tick_draw()` in the tab worker applies these optimisations in order:

1. **TileCache path** (`backend_cairo` or `backend_skia`): both backends share one unified block.
   - Scroll-only: `take_scroll_handle(dpr)` → submit `TileCache`, return. Stages 1–6 skipped entirely.
   - Full render: `rebuild_render_list_if_needed()` → `tile_cache_handle(dpr)` → submit `TileCache`, return. `RenderBackend::render()` is never called.
2. **Default path** (Vello, Null, or `pipeline` feature disabled): rebuild render list → `render_backend.render()` → `external_handle()` → submit frame.

## Feature flags

| Flag | What it enables |
|---|---|
| `pipeline` | Enables all 7 stages; required by all backend flags |
| `backend_cairo` | Cairo rasterizer (stage 6) + Cairo display-list compositor |
| `backend_cairo_pango` | Pango text layout inside the Cairo rasterizer; requires GTK4 |
| `backend_skia` | Skia CPU rasterizer (stage 6); host composites tiles directly |
| `backend_vello` | Vello/wgpu GPU rasterizer |

The `backend_skia_gl` feature on `gosub_render_pipeline` additionally enables `SkiaGpuBackend` for OpenGL compositing (used in `winit-skia-gpu`), but the engine pipeline stages still use `backend_skia`.

## Key constants

| Name | Value | Location | Purpose |
|---|---|---|---|
| Default tile size | 256 × 256 px | `context.rs` | Grid unit for stage 4 |
| `DEFAULT_FONT_SIZE` | 16.0 px | `layouter/taffy.rs` | Fallback when CSS font-size absent |
| `DEFAULT_FONT_FAMILY` | `"Sans"` | `layouter/taffy.rs` | Fallback font family |
| `DEVICE_PIXEL_RATIO` | `AtomicU32`, default 1 | `render/backends/cairo.rs` | Set by GTK display thread; scales Cairo tile surfaces |
| Invisible tags | `head style script meta link title` | `rendertree_builder/tree.rs` | Pruned from render tree before stage 1 |
