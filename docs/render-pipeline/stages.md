# Pipeline Stages

All seven stages are orchestrated in `crates/gosub_engine/src/engine/context.rs`.  
Stages 1–6 run inside `pipeline_build_cache()` and their output is cached in `PipelineCache`.  
Stage 7 runs every frame inside `pipeline_composite()` or is bypassed for the Skia path (see README).

Each stage is timed and its duration logged at `INFO` level with the `[pipeline]` prefix.

---

## Stage 1 — Render Tree Builder

**Module:** `crates/gosub_render_pipeline/src/rendertree_builder/`  
**Input:** `Arc<dyn PipelineDocument>`  
**Output:** `RenderTree`

Builds a pruned, flat-indexed view of the DOM that contains only nodes relevant to rendering.

### What gets filtered out

- Tag names in the invisible set: `head`, `style`, `script`, `meta`, `link`, `title`
- Comment nodes
- Elements with `display: none`

### Algorithm

`build_rendertree()` does an iterative post-order traversal using an explicit `Frame` stack (avoids stack overflow on deep documents). Each element node becomes a `RenderNode` in an arena; text nodes are preserved as children of their parent element.

Result: `HashMap<RenderNodeId, RenderNode>` with a single `root_id`. `RenderNodeId` is a newtype over `u64`.

---

## Stage 2 — Layout

**Module:** `crates/gosub_render_pipeline/src/layouter/taffy.rs`  
**Input:** `RenderTree`, viewport `Dimension`  
**Output:** `Arc<LayoutTree>`

Computes the final position and size of every render node using the [Taffy](https://github.com/DioxusLabs/taffy) CSS layout engine. Covered in depth in [layout.md](layout.md); the short version:

### Steps

1. **Tree conversion** — `generate_tree()` walks the `RenderTree` and builds a parallel `TaffyTree<TaffyContext>`. Each node carries a `TaffyContext` that tells the measure callback what kind of content it holds. Inline children (text, inline elements, `<br>`-separated line boxes) are wrapped in anonymous flex containers to emulate inline formatting, which Taffy lacks.
2. **CSS → Taffy** — `CssTaffyConverter` maps `StylePropertyList` values to Taffy's `Style` struct (flex, grid, box model, sizing, positioning, overflow, typography).
3. **Measurement callbacks** — Taffy calls back for intrinsic sizes: text nodes measure through the shared [font system](../fonts.md) (memoized, since Taffy probes each node 2–4×); image/SVG nodes honour CSS-constrained dimensions and derive the rest from their intrinsic aspect ratio. Image fetches are non-blocking — layout proceeds with placeholder sizes and a reflow lands when the media arrives.
4. **`populate_boxmodel()`** — Taffy's parent-relative results are converted to absolute page-space `BoxModel`s (margin / border / padding / content rects); after this the pipeline is layout-engine agnostic.
5. **Table post-processing** — `display: table` subtrees are re-laid-out by `gosub_lattice` in two passes (widths top-down, heights bottom-up for nested tables) and written back over the Taffy results.

### Output structure

```
LayoutTree
├── render_tree: RenderTree
├── arena: HashMap<LayoutElementId, LayoutElementNode>
│   └── LayoutElementNode
│       ├── box_model: BoxModel     (margin / border / padding / content rects)
│       ├── context: ElementContext (None | Text | Image | Svg)
│       └── children: Vec<LayoutElementId>
└── root_dimension: Dimension       (full page size after layout)
```

---

## Stage 3 — Layering

**Module:** `crates/gosub_render_pipeline/src/layering/`  
**Input:** `Arc<LayoutTree>`  
**Output:** `Arc<LayerList>`

Assigns layout elements to ordered layers for z-order compositing. Covered in depth in [layering-and-compositing.md](layering-and-compositing.md); the short version:

### Current behaviour

- Elements join the enclosing layer by default; the root starts a base layer at `order = 0`.
- An element is **promoted** to its own layer (subtree included) when it has `opacity < 1`, `position: fixed`, or `position: sticky` — effects the compositor applies per frame to cached tiles — or when a positioned element declares an explicit `z-index`.
- A promoted `Layer` carries its stacking `order` (from `z-index`), a group `opacity`, and a `TileAnchor` (`Scroll` / `Fixed` / `Sticky(StickyConstraint)`) describing how it responds to scroll at composite time.
- Standalone `<img>` elements outside a promoted group still get their own layer at their stacking level.
- After traversal, layers are stably sorted by `order`, so equal-`z-index` layers keep DOM order.

The `LayerList` also provides hover hit-testing via `find_element_at(vp_x, vp_y, scroll_x, scroll_y)`, which walks layers front-to-back and inverts each layer's anchor mapping (a fixed layer is tested at raw viewport coordinates, a scrolling one at `viewport + scroll`).

### Future work

`mix-blend-mode`, transforms, and nested opacity groups are not yet implemented; sticky supports only `top`/`left` insets. See the [limitations section](layering-and-compositing.md#current-limitations).

---

## Stage 4 — Tiling

**Module:** `crates/gosub_render_pipeline/src/tiler.rs`  
**Input:** `Arc<LayerList>`  
**Output:** `TileList`

Divides the full page into a uniform grid of tiles and maps elements to tiles they intersect.

### Tile grid

```
cols = ceil(page_width  / tile_width)   // default tile_width  = 256 px
rows = ceil(page_height / tile_height)  // default tile_height = 256 px
```

One grid is created per layer. Tiles are inserted into an **R\* tree** (`rstar::RTree`) for fast spatial queries.

### Element distribution

For each layout element, `get_intersecting_tiles()` queries the R\* tree and returns all tiles the element's bounding rect overlaps. For each intersecting tile a `TiledLayoutElement` is created:

- `rect` — the element's bounding box clipped to this tile
- `position` — element origin relative to tile top-left
- `paint_commands` — filled in stage 5

The background colour is extracted from the `<html>` or `<body>` element and stored in `Tile.bgcolor` so the rasterizer can clear each tile to the page background before drawing content.

### `TileState`

| State | Meaning |
|---|---|
| `Dirty` | Needs rasterization |
| `Clean` | Pixel data is valid; has a `texture_id` |
| `Empty` | No visible content; skip compositing |
| `Unrenderable` | Backend cannot rasterize this tile |

---

## Stage 5 — Painting

**Module:** `crates/gosub_render_pipeline/src/painter/`  
**Input:** `TileList` (elements without paint commands)  
**Output:** `TileList` (elements with `Vec<PaintCommand>`)

Converts each `TiledLayoutElement` into a backend-agnostic sequence of draw commands. No pixels are produced at this stage.

### Command generation

`Painter::paint(element, browser_state)` inspects `ElementContext` and `BoxModel`:

| Element kind | Commands emitted |
|---|---|
| Text node | `PaintCommand::Text { text, font_info, brush, rect }` |
| `<img>` | `PaintCommand::Rectangle` with `Brush::Image(media_id)` |
| `<svg>` | `PaintCommand::Svg { media_id, rect }` |
| Everything else | `PaintCommand::Rectangle` with background colour, border, radius |

Optional debug overlays (hover box-model, wireframe) are also added here when `BrowserState` flags are set.

### Paint command types

```rust
PaintCommand::Rectangle(Rectangle)   // fill + stroke a rect; optional border-radius
PaintCommand::Text(Text)             // string, FontInfo, colour brush, bounding rect
PaintCommand::Svg(PaintSvg)         // reference to a loaded SVG in MediaStore
```

---

## Stage 6 — Rasterization

**Module:** `crates/gosub_render_pipeline/src/rasterizer.rs` (trait)  
**Implementations:** `gosub_renderer_cairo`, `gosub_renderer_skia`  
**Input:** `TileList` + `MediaStore`  
**Output:** `TextureStore` (pixel buffers keyed by `TextureId`)

Executes paint commands and produces raw pixel buffers. The rasterizer is provided by the configured backend at runtime: the engine calls `RenderBackend::create_rasterizer()` once and drives it per the backend's `RasterStrategy`.

### Rasterable trait

```rust
pub trait Rasterable {
    fn rasterize(
        &self,
        tile: &Tile,
        texture_store: &mut TextureStore,
        media_store: &MediaStore,
    ) -> Option<TextureId>;
}
```

Returns `None` for tiles with no renderable content (mapped to `TileState::Empty`).

### Cairo rasterizer (`crates/gosub_renderer_cairo`)

Selected by naming `CairoBackend` in the config (see [../configuration.md](../configuration.md)).

- **DPR:** reads `DEVICE_PIXEL_RATIO` static atomic (set by the GTK display thread from `area.scale_factor()`).
- **Surface size:** `tile_css_width × DPR` by `tile_css_height × DPR` physical pixels.
- **Context:** creates a `cairo::Context`, scales it by DPR so all CSS-pixel coordinates map to physical pixels, then dispatches commands:
  - `Rectangle` → `rectangle::do_paint_rectangle()` — path + fill; handles borders and border-radius.
  - `Text` → `text::pango::do_paint_text()` with Pango (the `text_pango` feature, default) or Parley — see [../fonts.md](../fonts.md).
  - `Svg` → `svg::do_paint_svg()` via librsvg.
- **Output:** premultiplied ARGB32 pixel data (`cairo::Format::ARgb32`), stride = `tile_phys_width × 4`.

### Skia rasterizer (`crates/gosub_renderer_skia`)

Selected by naming `SkiaBackend` in the config (see [../configuration.md](../configuration.md)).

- **DPR:** reads the global `DEVICE_PIXEL_RATIO` atomic at rasterize time (like Cairo).
- **Surface size:** `tile_css_width × DPR` by `tile_css_height × DPR` physical pixels; the canvas is scaled by DPR so paint commands stay in CSS coordinates.
- **Context:** creates a `skia_safe` raster surface, clips to tile bounds, pre-translates canvas by `-tile.rect.x, -tile.rect.y` so paint commands work in page coordinates, then dispatches:
  - `Rectangle` → `rectangle::do_paint_rectangle()` — `draw_rect` / `draw_round_rect`; handles solid fills and borders.
  - `Text` → `text::do_paint_text()` — word-wraps via `font.measure_str()`, renders with `draw_str()`.
  - `Svg` → `svg::do_paint_svg()`.
- **Output:** premultiplied BGRA8888 (a `surfaces::raster` surface with an explicit `BGRA8888`/`Premul` `ImageInfo`), stride = `tile_phys_width × 4` — byte-for-byte compatible with Cairo's `ARgb32`.

### Pixel format compatibility

Both rasterizers produce `BGRA8888` (premultiplied) in memory on little-endian systems. Cairo calls this `ARgb32`; Skia calls it `n32_premul`; they are the same byte layout. This means the same `DisplayItem::Blit` handling code works for both backends.

---

## Stage 7 — Compositing

**Location:** `pipeline_composite()` in `crates/gosub_engine/src/engine/context.rs`  
**Input:** `PipelineCache` + scroll offset + viewport size  
**Output:** `RenderList` populated with `DisplayItem::Blit`

The final stage of the display-list path selects visible tiles from the cache and emits blit items for the render backend. It runs every frame; cost is proportional to visible tile count and is typically sub-millisecond.

### Algorithm

```rust
rl.push(DisplayItem::Clear { color: WHITE });
for tile in cache.tiles {
    // Cull tiles fully outside the viewport
    if tile.page_x + tile.width  <= scroll_x { continue; }
    if tile.page_y + tile.height <= scroll_y { continue; }
    if tile.page_x >= scroll_x + vp_width    { continue; }
    if tile.page_y >= scroll_y + vp_height   { continue; }

    rl.push(DisplayItem::Blit {
        x: (tile.page_x - scroll_x) as f32,
        y: (tile.page_y - scroll_y) as f32,
        w: tile.width,
        h: tile.height,
        data: (*tile.data).clone(),   // Arc<Vec<u8>>, premultiplied BGRA32
    });
}
```

### Skia bypass (TileCache path)

With the Skia backend, `pipeline_composite()` and `RenderBackend::render()` are not called. Instead the engine calls `tile_cache_handle(dpr)` which wraps the same `Arc<Vec<CachedTile>>` into an `ExternalHandle::TileCache` and submits it to the compositor. The host window thread composites the tiles directly — for example, by uploading each tile as a Skia image and calling `draw_image()` on a GPU canvas.

---

## Dirty flags and caching

`BrowsingContext` tracks several granular dirty flags:

| Flag | Set when | Triggers |
|---|---|---|
| `render_dirty` | DOM change, style change, layout change, viewport resize | Full stages 1–6 rebuild |
| `scroll_dirty` | Scroll offset changes | Stage 7 only (or TileCache fast path) |
| `dom_dirty` | HTML content changed | Sets `render_dirty` |
| `style_dirty` | CSS computed values changed | Sets `render_dirty` |
| `layout_dirty` | Box model changed | Sets `render_dirty` |

`rebuild_render_list_if_needed()` checks `render_dirty` first; if true it calls `pipeline_build_cache()`, clears all flags, then runs stage 7. If only `scroll_dirty` is true it skips stages 1–6 and runs stage 7 against the existing cache.

### Scroll fast paths

**Cairo**: `take_scroll_handle(dpr)` returns `Some(TileCache)` only when `scroll_dirty && !render_dirty`. The tab worker submits the handle immediately (in the scroll event handler, not waiting for the next tick) to eliminate up to 33 ms of latency at 30 fps.

**Skia**: `tile_cache_handle(dpr)` is called unconditionally after `rebuild_render_list_if_needed()`. The TileCache is always submitted regardless of what triggered the dirty flag.

### Hover hit-testing

`BrowsingContext::update_hover(vp_x, vp_y)` uses the cached `LayerList` to find the DOM node under the cursor without re-running any pipeline stage. It walks ancestor nodes to detect `<a href>` links and emits `EngineEvent::HoverUrl`.
