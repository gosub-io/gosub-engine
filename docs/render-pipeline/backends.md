# Render Backends

A render backend is responsible for consuming the `RenderList` produced by
Stage 7 and drawing it to a visible surface (a window, an off-screen buffer, or
a PNG file).

Backends are **decoupled from the pipeline**: they see only `DisplayItem` enums
and have no knowledge of tiles, textures, or paint commands.

---

## The `RenderBackend` trait

**File:** `crates/gosub_engine/src/render/backend.rs`

```rust
pub trait RenderBackend {
    fn name(&self) -> &'static str;
    fn create_surface(&self, size: SurfaceSize, present: PresentMode)
        -> Result<Box<dyn ErasedSurface + Send>>;
    fn render(&self, ctx: &mut BrowsingContext, surface: &mut dyn ErasedSurface)
        -> Result<()>;
    fn snapshot(&self, surface: &mut dyn ErasedSurface, max_dim: u32)
        -> Result<RgbaImage>;
    fn external_handle(&self, surface: &mut dyn ErasedSurface)
        -> Result<ExternalHandle>;
}
```

`render()` reads `ctx.render_list()` and draws each `DisplayItem` onto the
`ErasedSurface`. After drawing, the surface can be presented to a window or
read back via `snapshot()`.

---

## Cairo backend

**Crate:** `crates/gosub_cairo/`  
**Feature:** `backend_cairo`  
**Surface type:** `OffscreenCairoSurface` (CPU, ARgb32 pixel buffer)

### How it draws each `DisplayItem`

| Variant | Cairo operation |
|---------|----------------|
| `Clear { color }` | `set_operator(Source)` + `paint()` — replaces entire surface |
| `Rect { … }` | `rectangle()` + `fill()` |
| `TextRun { … }` | `select_font_face()` + `show_text()` (basic fallback, no Pango) |
| `Blit { x, y, w, h, data }` | `create_for_data_unsafe()` → `SurfacePattern` → `fill()` |

The `Blit` path is the primary path when the pipeline is active. It:

1. Validates `data.len() >= h * w * 4` before touching the pointer.
2. Creates a read-only `cairo::ImageSurface` over the existing buffer (zero
   copy — Cairo never writes to source data).
3. Builds a `SurfacePattern` with a translation matrix that maps from page
   space into surface space.
4. Fills the tile rectangle with the pattern.

### Thread safety

`OffscreenCairoSurface` is `Send`. The `CairoBackend` struct is stateless and
can be shared across threads.

### GTK integration

The `gosub_cairo` crate also provides a GTK4 drawing area integration (in
`src/lib.rs`) that calls `render()` on the `draw` signal and uses
`ExternalHandle` to hand the pixel buffer to GDK.

---

## Vello backend

**Crate:** `crates/gosub_vello/`  
**Feature:** `backend_vello`  
**Surface type:** GPU texture via wgpu

### How it draws each `DisplayItem`

| Variant | Vello operation |
|---------|----------------|
| `Clear { color }` | Clears the wgpu render pass |
| `Rect { … }` | `scene.fill()` with a solid brush |
| `TextRun { … }` | Logs a warning; not yet implemented |
| `Blit { … }` | Logs a warning; not yet implemented |

The Vello backend currently has its own internal rasterization path (separate
from the `TextureStore` blit approach used by Stage 7). Full integration with
the compositing stage is pending.

### wgpu surface lifecycle

`VelloSurface` wraps a `wgpu::Surface` and a `wgpu::Texture`. `create_surface`
creates the wgpu resources; `present()` submits the render pass and presents
the swap chain.

---

## Null backend

**File:** `crates/gosub_engine/src/render/backends/null.rs`  
**Always available** (no feature flag required)

The null backend does nothing. It is used in headless tests and CI where no
display server is available. `render()` is a no-op; `snapshot()` returns a
1×1 transparent image.

---

## Adding a new backend

1. Implement `RenderBackend` for your type.
2. Implement `ErasedSurface` for your surface type.
3. Handle at minimum `DisplayItem::Clear` and `DisplayItem::Blit`.
4. Wire it up in `BrowsingContext::new()` behind a feature flag.

The pipeline's output is a flat `Vec<DisplayItem>` — no Rust generics or
associated types leak into the backend interface. Any backend that can blit
an ARgb32 buffer can be integrated without touching the pipeline crates.

---

## Pixel format

All `Blit` payloads and `OffscreenCairoSurface` pixel buffers use the same
format:

- **Format:** `cairo::Format::ARgb32` (also known as `BGRA8888` on
  little-endian, with premultiplied alpha)
- **Stride:** `width * 4` bytes
- **Total size:** `height * width * 4` bytes

When blitting to a different format (e.g. wgpu `Bgra8UnormSrgb`), the backend
is responsible for any format conversion.
