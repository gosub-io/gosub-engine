# Render Pipeline

The Gosub render pipeline transforms a parsed HTML document into pixel data displayed in a window. It is a **staged, tile-based pipeline** that caches rasterized tiles across frames and only re-runs expensive stages when content actually changes.

The pipeline is always compiled into `gosub_engine` (it is a plain dependency, not a Cargo feature). What varies is the render backend: it is chosen as a config type (see [configuration.md](../configuration.md)), and a backend's capability queries (`raster_strategy()`, `renders_to_gpu_texture()`) decide at runtime which pipeline path a tab uses. With the `NullBackend` the stages up to rasterization still run — there is simply no rasterizer, so no tiles or pixels are produced.

The pipeline uses its own document/style/layout types, separate from the `gosub_interface` world where parsing happens — see [The two worlds](../two-worlds.md) for the split and the adapter that joins them.

## Quick navigation

| Document | Contents |
|---|---|
| [Stages](stages.md) | Deep dive into each of the 7 pipeline stages |
| [Data structures](data-structures.md) | Key types and how they flow between stages |
| [Layout](layout.md) | Stage 2 in depth: Taffy integration, inline emulation, text measurement, tables via lattice |
| [Layering & compositing](layering-and-compositing.md) | Layer promotion (opacity, fixed, sticky, z-index) and how the compositor realises it |
| [Backends](backends.md) | Render backends, ExternalHandle, and host compositing |
| [GPU render flow](gpu-render-flow.md) | CPU tile flow vs. GPU one-shot scene flow |

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
               │  rebuild_pipeline_cache_if_needed() / tile_cache_handle()
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

1. **TileCache path** (backends whose `raster_strategy()` is not `None` and that don't render to a GPU texture — Cairo and Skia): one unified block.
   - Scroll-only: `take_scroll_handle(dpr)` → submit `TileCache`, return. Stages 1–6 skipped entirely.
   - Full render: `rebuild_pipeline_cache_if_needed()` (stages 1–6, no display list) → `tile_cache_handle(dpr)` → submit `TileCache`, return. `RenderBackend::render()` is never called.
2. **Default path** (Vello, Null): rebuild render list → `render_backend.render()` → `external_handle()` → submit frame. GPU backends with `renders_to_gpu_texture()` render from the paint scene here (see [gpu-render-flow.md](gpu-render-flow.md)).

## Feature flags

Backend selection is **not** a feature flag — backends are separate crates named as config types ([configuration.md](../configuration.md)). The features that do exist:

| Crate | Flag | What it enables |
|---|---|---|
| `gosub_render_pipeline` | `parley_layout` | Parley shaping path in text layout (default via `gosub_engine`) |
| `gosub_render_pipeline` | `wayland` / `x11` | GDK platform integration |
| `gosub_renderer_cairo` | `text_pango` (default) / `text_parley` / `text_skia` | which text rasterizer paints glyphs — see [fonts.md](../fonts.md) |
| `gosub_renderer_vello` | `text_parley` (default) / `text_skia` / `text_pango` | same, for Vello |
| `gosub_renderer_vello` | `parley_layout` | Parley shaping in the Vello text renderer (skrifa fallback when off) |

## Key constants

| Name | Value | Location | Purpose |
|---|---|---|---|
| Default tile size | 256 × 256 px | `context.rs` | Grid unit for stage 4 |
| `DEFAULT_FONT_SIZE` | 16.0 px | `layouter/taffy.rs` | Fallback when CSS font-size absent |
| `DEFAULT_FONT_FAMILY` | `"sans-serif"` | `layouter/taffy.rs` | Fallback font family |
| `DEVICE_PIXEL_RATIO` | `AtomicU32`, default 1 | `gosub_interface/src/render/viewport.rs` | Set by the display thread; scales Cairo/Skia tile surfaces |
| Invisible tags | `head style script meta link title` | `rendertree_builder/tree.rs` | Pruned from render tree before stage 1 |
