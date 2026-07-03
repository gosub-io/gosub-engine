# Render Backends

A render backend turns the engine's output into pixels on screen. There are two distinct compositing paths:

## Compositing paths

### Display-list path

Stage 7 (`pipeline_composite`) assembles visible tiles into a `RenderList` of `DisplayItem::Blit` entries. The engine calls `render_backend.render()` which processes that list and writes pixels into an off-screen surface. `external_handle()` then returns a `CpuPixelsOwned` (or similar) handle that the host reads back to display.

**Used by:** Cairo (always), Vello.

### TileCache path

Stage 7 is bypassed entirely. After stages 1–6 build the `PipelineCache`, the engine calls `tile_cache_handle()` and submits an `ExternalHandle::TileCache` directly to the compositor. `RenderBackend::render()` is **never called**. The host window thread receives a list of `Arc<Vec<u8>>` tiles and composites them itself — in a GTK draw callback, a winit softbuffer blit loop, or a GPU canvas on the main GL thread.

**Used by:** all Skia backends (always, regardless of display-list backend also being wired up).

### Scroll-only fast path (Cairo and Skia)

When only the scroll offset changed (no content or layout change), `take_scroll_handle()` returns a `TileCache` immediately without rebuilding any pipeline stage. This eliminates up to 33 ms of render latency at 30 fps and is available for both Cairo and Skia backends.

### Note on `SkiaBackend::render()`

`SkiaBackend` (in `crates/gosub_renderer_skia/src/backend.rs`) is wired up as the `RenderBackend` in all Skia examples and does implement `render()` against the display list. However, because its `raster_strategy()` is `ParallelCached` (and it does not render to a GPU texture), `tick_draw` returns early via the TileCache path and never reaches `render_backend.render()`. `SkiaBackend::render()` is therefore dead code in normal operation — it exists as a fallback for headless snapshot use.

---

## ExternalHandle

`ExternalHandle` is the typed envelope that the compositor hands to the host window thread. The host pattern-matches on it to decide how to draw.

```rust
pub enum ExternalHandle {
    NullHandle        { width, height, frame_id },
    CpuPixelsOwned    { width, height, stride, pixels: Vec<u8>, format: PixelFormat },
    CpuPixelsPtr      { width, height, stride, pixel_buf: NonNull<u8> },
    TileCache         { viewport_width, viewport_height, dpr,
                        scroll_x, scroll_y, page_height,
                        tiles: Arc<Vec<CachedTile>> },
    GlTexture         { tex, target, width, height, frame_id },
    WgpuTextureId     { id, width, height, format, frame_id },
    SkiaImageId       { id, width, height, frame_id },
    GlFramebufferRendered { frame_id },
}
```

### CachedTile

```rust
pub struct CachedTile {
    pub page_x:  f32,      // tile origin in page space (CSS pixels)
    pub page_y:  f32,
    pub width:   u32,      // tile physical pixel width
    pub height:  u32,
    pub data:    Arc<Vec<u8>>,   // premultiplied BGRA32, stride = width × 4
}
```

`Arc` ownership lets the host thread hold tiles across frames with zero copies, even while the engine is building the next cache.

### TileCache scroll fields

`scroll_x` / `scroll_y` in `TileCache` record the engine's scroll offset at the time the handle was created. The host uses them to compute screen positions:

```
screen_x = tile.page_x - scroll_x
screen_y = tile.page_y - scroll_y
```

Tiles outside `[0, viewport_width) × [0, viewport_height)` after this transform should be culled before drawing.

---

## The `RenderBackend` trait

**File:** `crates/gosub_interface/src/render/backend.rs` (re-exported via `gosub_render_pipeline::render::backend`)

```rust
pub trait RenderBackend: Send {
    fn name(&self) -> &'static str;
    fn create_surface(&self, size: SurfaceSize, present: PresentMode)
        -> Result<Box<dyn ErasedSurface + Send>>;
    fn render(&self, ctx: &mut dyn RenderContext, surface: &mut dyn ErasedSurface)
        -> Result<()>;
    fn snapshot(&self, surface: &mut dyn ErasedSurface, max_dim: u32)
        -> Result<RgbaImage>;
    fn external_handle(&self, surface: &mut dyn ErasedSurface)
        -> Result<ExternalHandle>;
}
```

`render()` reads `ctx.render_list()` and draws each `DisplayItem` onto the `ErasedSurface`. When the pipeline is in TileCache mode (Skia path) this method is never called.

---

## Cairo backend

**Crate:** `crates/gosub_renderer_cairo` (backend in `src/backend.rs`)  
**Surface format:** premultiplied ARgb32 (= BGRA8888 on little-endian)

### Surface creation

`CairoBackend::create_surface()` reads the `DEVICE_PIXEL_RATIO` atomic and creates a physical-pixel surface:

```
physical_width  = css_width  × DPR
physical_height = css_height × DPR
```

The atomic is set by the GTK display thread before any rendering begins:

```rust
DEVICE_PIXEL_RATIO.store(area.scale_factor() as u32, Ordering::Relaxed);
```

### Rendering display list items

| DisplayItem | Cairo operation |
|---|---|
| `Clear { color }` | `set_operator(Source)` + `paint()` — replaces entire surface |
| `Rect { … }` | `rectangle()` + `fill()` |
| `TextRun { … }` | `select_font_face()` + `show_text()` (basic fallback) |
| `Blit { x, y, w, h, data }` | Creates a read-only `ImageSurface` over the tile buffer; builds a `SurfacePattern` with translation matrix; fills the tile rectangle |

All CSS-pixel coordinates in `Blit` are multiplied by DPR before use, so tiles rasterized at physical resolution land at their correct screen positions.

### External handle

`external_handle()` returns `CpuPixelsOwned` when called via the display-list path (e.g. headless snapshot). In normal interactive rendering `render()` and `external_handle()` are never called — the engine uses the TileCache path instead (see below).

### TileCache path (interactive rendering)

For all interactive rendering, the Cairo backend uses the unified TileCache path shared with Skia. The engine submits `ExternalHandle::TileCache` directly; the host composites tiles without invoking `CairoBackend::render()`.

On GTK, `draw_tile_cache()` blits each tile using `cairo::ImageSurface::create_for_data_unsafe()` — zero-copy, Cairo reads source data but never writes it. On winit, tiles are blitted into the softbuffer pixel loop.

---

## Skia CPU backend

**Crate:** `crates/gosub_renderer_skia` (backend in `src/backend.rs`)  
**Surface format:** premultiplied BGRA8888

### Pipeline integration

The Skia backend **bypasses the display-list render pipeline entirely**. After stages 1–6 produce a `PipelineCache`, the engine calls `tile_cache_handle(dpr)` (with `dpr = render_backend.device_pixel_ratio()`, which reads the `DEVICE_PIXEL_RATIO` atomic) and submits `TileCache` directly to the compositor. The host composites tiles on its own thread (where the GL context is current if GPU compositing is used).

`SkiaBackend::render()` and `external_handle()` exist and return `CpuPixelsOwned` for cases where the display-list path is needed (e.g. egui integration), but they are not called in the normal Skia render loop.

### Rasterizer

`SkiaRasterizer` (in `crates/gosub_renderer_skia/src/rasterizer.rs`) runs during stage 6:

- Creates a `skia_safe::surfaces::raster` surface (explicit `BGRA8888`/`Premul` `ImageInfo`) sized to the tile in CSS pixels × `DEVICE_PIXEL_RATIO`.
- Pre-translates the canvas by `(-tile.rect.x, -tile.rect.y)` so paint commands work in page coordinates.
- Dispatches `PaintCommand` variants:
  - `Rectangle`: `draw_rect` / `draw_round_rect` with solid colour fills and stroke borders.
  - `Text`: greedy word-wrap using `font.measure_str()`; renders lines with `draw_str()`.
  - `Svg`: delegates to `svg::do_paint_svg()`.
- Reads pixels via `canvas.peek_pixels()` and stores them in the `TextureStore`.

### Host compositing (GTK / winit / egui)

The host receives `TileCache` and iterates the tile list:

```
for tile in tiles:
    screen_x = tile.page_x - scroll_x
    screen_y = tile.page_y - scroll_y
    [cull if outside viewport]
    [blit tile.data at (screen_x, screen_y)]
```

Blit implementations by host:
- **GTK4 (`gtk4-skia`)**: `cairo::ImageSurface::create_for_data_unsafe()` + `cr.set_source_surface()` + `cr.paint()`.
- **winit (`winit-skia`)**: pixel-copy loop into softbuffer `u32` framebuffer.
- **winit GPU (`winit-skia-gpu`)**: upload each tile as a `skia_safe::Image` via `raster_from_data()`, then `canvas.draw_image()` on a GL-backed Skia canvas, swap buffers.

---

## Skia with GPU compositing (`winit-skia-gpu`, `gtk4-skia-gpu`)

**Backend:** the regular `SkiaBackend` — there is no separate GPU backend type.  
**Where the GPU work lives:** the host example (`examples/winit-skia-gpu/main.rs`)

### How it works

The engine side is identical to the Skia CPU setup: `SkiaBackend` rasterizes tiles on the
CPU (BGRA32 buffers) and the tab worker ships them out as an `ExternalHandle::TileCache`.
What changes is the *host's* compositing: instead of blitting tiles into a CPU softbuffer,
the example composites them on the GPU through Skia's Ganesh/GL backend.

OpenGL contexts are thread-affine; in `winit-skia-gpu` the GL context is created on and
stays current on the main (event-loop) thread, where all compositing happens. Each redraw
wraps the window's default framebuffer (FBO 0) in a Skia GPU surface:

```rust
let render_target = backend_render_targets::make_gl((w, h), None, 8, fb_info);
let surface = surface_ganesh::wrap_backend_render_target(
    &mut direct_context, &render_target, BottomLeft, RGBA8888, ..,
);
```

then uploads each cached tile as a `skia_safe::Image` and draws it with
`canvas.draw_image()`. After all tiles and the address bar are drawn,
`direct_context.flush_and_submit()` + `swap_buffers()` presents to the window.

**No CPU readback occurs.** Tile pixels go directly from CPU → GPU texture → framebuffer.

---

## Vello backend

**Crate:** `crates/gosub_renderer_vello` (backend in `src/backend.rs`)  
**Used by:** `egui-vello`, `winit-vello`

Vello is a GPU vector-graphics renderer built on wgpu. It processes the display list differently from the Blit-based backends: it rebuilds a `Scene` every frame from the `RenderList` rather than consuming pre-rasterized pixel tiles.

### Display item handling

| DisplayItem | Vello operation |
|---|---|
| `Clear { color }` | Clear the wgpu render pass |
| `Rect { … }` | `scene.fill()` with solid brush |
| `TextRun { … }` | Rendered via internal font cache + text renderer |
| `Blit { … }` | Not yet implemented (warns, skips) |

`Blit` is currently unimplemented, which means Vello cannot consume the tile-based output of the standard pipeline. The Vello path uses a separate internal rasterization approach.

### wgpu integration

`VelloBackend` requires a `WgpuContextProvider` that supplies a `wgpu::Device`, `wgpu::Queue`, and target texture. The `render()` method submits a wgpu render pass; `external_handle()` returns `WgpuTextureId` or `GlFramebufferRendered`.

---

## Null backend

**File:** `crates/gosub_render_pipeline/src/render/backends/null.rs`  
**Always available** (no feature flag)

Used when the engine needs a valid backend but no rendering is desired (headless, screenshot pipeline when only tile data is needed). `render()` is a no-op; `external_handle()` returns `NullHandle`.

---

## Backend × example mapping

| Example | Framework | Backend | TileCache path? | Storage |
|---|---|---|---|---|
| `gtk4-cairo` | GTK4 | Cairo | Yes (scroll fast path) | Persistent SQLite |
| `gtk4-skia` | GTK4 | Skia CPU | Yes (always) | Persistent SQLite |
| `gtk4-skia-gpu` | GTK4 + GL | Skia GPU | Yes (always) | Persistent SQLite |
| `winit-cairo` | winit + softbuffer | Cairo | Yes (scroll fast path) | In-memory |
| `winit-skia` | winit + softbuffer | Skia CPU | Yes (always) | In-memory |
| `winit-skia-gpu` | winit + glutin/GL | Skia GPU | Yes (always, GPU blit) | In-memory |
| `winit-vello` | winit + wgpu | Vello | No (display list) | In-memory |
| `egui-cairo` | egui/eframe | Cairo | No (display list) | In-memory |
| `egui-skia` | egui/eframe | Skia CPU | Yes (always) | In-memory |
| `egui-vello` | egui/eframe | Vello | No (display list) | In-memory |
| `gosub-screenshot` (bin) | Headless | Skia CPU | Yes (tiles composited in-process → PNG) | In-memory |

---

## Adding a new backend

1. Implement `RenderBackend` for your type, with `ErasedSurface` for the surface.
2. Handle `DisplayItem::Clear` and `DisplayItem::Blit` at minimum. Pixel data in `Blit` is premultiplied BGRA32.
3. Return an appropriate `ExternalHandle` variant from `external_handle()`.
4. Wire it up in your example's `main()` behind a feature flag.

If your backend should use the **TileCache path** (pre-rasterized CPU tiles composited by the host), no engine changes are needed: return `RasterStrategy::ParallelCached` from `raster_strategy()`, provide a rasterizer via `create_rasterizer()`, and leave `renders_to_gpu_texture()` at `false` — `tick_draw()` selects the path from those capability queries.
