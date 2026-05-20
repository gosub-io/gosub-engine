# Pipeline Stages

All seven stages run inside `pipeline_render()` in
`crates/gosub_engine/src/engine/context.rs`. Each stage is timed and its
duration is logged at `INFO` level with the `[pipeline]` prefix.

---

## Stage 1 — Render Tree Builder

**Crate/module:** `crates/gosub_pipeline/src/rendertree_builder/`  
**Input:** `Arc<dyn PipelineDocument>`  
**Output:** `RenderTree`

The render tree is a pruned, flat-indexed copy of the DOM that contains only
nodes relevant to rendering. Anything the CSS engine would also skip is dropped
here before layout gets involved.

### What gets filtered out

- Elements whose tag name is in the *invisible set*:
  `head`, `style`, `script`, `meta`, `link`, `title`
- Comment nodes
- Elements with `display: none`

### Algorithm

`build_rendertree()` does an iterative post-order traversal using an explicit
`Frame` stack (avoids stack overflow on deep documents). For each element node
it pushes a `RenderNode` into the arena; for text nodes it preserves them as
children of their parent element.

The result is a `HashMap<RenderNodeId, RenderNode>` with a single `root_id`.
`RenderNodeId` is a newtype over `u64` and is convertible to/from the
interface-level `NodeId`.

---

## Stage 2 — Layout

**Crate/module:** `crates/gosub_pipeline/src/layouter/taffy.rs`  
**Input:** `RenderTree`, viewport `Dimension`  
**Output:** `LayoutTree`

Layout computes the final position and size of every render node using the
[Taffy](https://github.com/DioxusLabs/taffy) CSS layout engine.

### Steps

1. **Tree conversion** — `generate_tree()` walks the `RenderTree` and creates
   a parallel `TaffyTree<TaffyContext>`. Each node carries a `TaffyContext`
   that tells the measure callback what kind of content it holds.

2. **CSS → Taffy** — `CssTaffyConverter` maps pipeline `StylePropertyList`
   values to Taffy's `Style` struct. Supported CSS properties include flex,
   grid, box model, sizing, positioning, overflow, and typography basics.

3. **Measurement callbacks** — Taffy calls back into Gosub for intrinsic sizes:
   - *Text nodes* call `get_text_layout()` with the font descriptor and
     available width, returning a pixel `Size`.
   - *Image/SVG nodes* return the intrinsic `Dimension` stored in
     `ElementContextImage` / `ElementContextSvg`.

4. **`populate_boxmodel()`** — After Taffy finishes, results are read back and
   stored in `LayoutElementNode.box_model` as a `BoxModel` with four `Edges`
   (margin, border, padding, content).

### `LayoutTree` structure

```
LayoutTree
├── render_tree: RenderTree         (source)
├── arena: HashMap<LayoutElementId, LayoutElementNode>
│   └── LayoutElementNode
│       ├── box_model: BoxModel     (margin/border/padding/content)
│       ├── context: ElementContext (None | Text | Image | Svg)
│       └── children: Vec<LayoutElementId>
└── root_dimension: Dimension       (viewport extent after layout)
```

---

## Stage 3 — Layering

**Crate/module:** `crates/gosub_pipeline/src/layering/`  
**Input:** `Arc<LayoutTree>`  
**Output:** `Arc<LayerList>`

Layering assigns layout elements to ordered *layers* for correct z-order
compositing. The current implementation is intentionally minimal.

### Current behaviour

- All elements go into a **default layer** at `order = 0`.
- Any element corresponding to an `<img>` tag gets its own dedicated layer at
  `order = 1`, ensuring images render on top of normal content.

### Future work

Full CSS stacking context support (z-index, `position: fixed`, opacity layers,
mix-blend-mode) is not yet implemented. The architecture has a clean
`LayerList` / `Layer` abstraction ready for it.

---

## Stage 4 — Tiling

**Crate/module:** `crates/gosub_pipeline/src/tiler.rs`  
**Input:** `Arc<LayerList>`, viewport rect  
**Output:** `TileList`

The viewport is divided into a uniform grid of tiles. Each tile represents a
fixed-size region (default: **256 × 256 pixels**) of page space. Elements are
clipped and distributed to whichever tiles they intersect.

### Tile grid construction

```
cols = ceil(viewport_width  / tile_width)
rows = ceil(viewport_height / tile_height)
```

One grid is created per layer. Tiles are inserted into an **R* tree**
(`rstar::RTree`) for fast spatial queries.

### Element distribution

For each layout element, `get_intersecting_tiles()` queries the R* tree and
returns all tiles the element's bounding rect overlaps. For each intersecting
tile, a `TiledLayoutElement` is created:

- `rect` — the element's bounding box clipped to the tile boundary
- `position` — where the element origin falls inside the tile
- `paint_commands` — filled in Stage 5

### `TileState`

Each tile starts with `state = Dirty`. After rasterization it becomes `Clean`
(has a texture) or `Empty` (no visible content, skip compositing).

---

## Stage 5 — Painting

**Crate/module:** `crates/gosub_pipeline/src/painter/`  
**Input:** `TileList` (elements without paint commands)  
**Output:** `TileList` (elements with `Vec<PaintCommand>`)

The painter converts each `TiledLayoutElement` into a sequence of backend-
independent draw commands. At this stage no pixels are produced — painting
is purely a data transformation.

### Command generation

`Painter::paint(element, browser_state)` is called for each element in each
tile. It inspects the element's `ElementContext` and `BoxModel` to decide which
commands to emit:

| Element kind | Command emitted |
|---|---|
| Text node | `PaintCommand::Text { text, font_info, brush, rect }` |
| `<img>` | `PaintCommand::Rectangle` with `Brush::Image(media_id)` |
| `<svg>` | `PaintCommand::Svg { media_id, rect }` |
| Everything else | `PaintCommand::Rectangle` with background color, border, radius |

### Paint command types

```rust
pub enum PaintCommand {
    Text(Text),
    Rectangle(Rectangle),
    Svg(PaintSvg),
}
```

A `Rectangle` carries:
- `rect: Rect` — position and size
- `brush: Brush` — fill (`Solid(Color)` or `Image(MediaId)`)
- `border: Option<Border>` — per-side width, style, color
- `radius: Option<Radius>` — per-corner border radius

A `Text` carries the string, `FontInfo` (family, size, weight), colour brush,
and the bounding rect.

---

## Stage 6 — Rasterization

**Crate/module:** `crates/gosub_pipeline/src/rasterizer/`  
**Input:** `TileList`, `&MediaStore`  
**Output:** `TextureStore` (pixel buffers keyed by `TextureId`)

Rasterization executes the paint commands and produces raw ARGB32 pixel buffers.
The rasterizer backend is selected at compile time via feature flags.

### Cairo rasterizer (`rasterizer/cairo.rs`)

1. Create a `cairo::ImageSurface` (ARgb32 format) sized to the tile dimensions.
2. Create a `cairo::Context` from the surface.
3. For each paint command in the tile:
   - `Rectangle` → `rectangle::do_paint_rectangle()` — sets source colour or
     image brush, draws path, fills; handles borders and rounded corners.
   - `Text` → `text::pango::do_paint_text()` (or Parley/Skia stub) — renders
     text via Pangocairo into an off-screen surface, then blits.
   - `Svg` → `svg::do_paint_svg()` — loads SVG from `MediaStore`, renders via
     librsvg.
4. Flush the surface, copy pixel data to a `Vec<u8>`, store in `TextureStore`.

All coordinates inside the rasterizer are in **tile-local space** (origin at
tile top-left). The tiler's `position` field provides the tile-relative offset
for each element.

### Vello rasterizer (`rasterizer/vello.rs`)

GPU-accelerated path using wgpu. Paint commands are translated to Vello scene
graph nodes and submitted to the GPU. Texture readback populates the same
`TextureStore` interface so Stage 7 is backend-agnostic.

### Text backends

| Feature flag | Library | Notes |
|---|---|---|
| `text_pango` | Pangocairo | Requires GTK4; production quality |
| `text_parley` | Parley | Pure Rust; default |
| `text_skia` | Skia | Stub, not yet implemented |

When multiple text features are enabled, `text_pango` takes precedence.

---

## Stage 7 — Compositing

**Location:** `pipeline_render()` in `crates/gosub_engine/src/engine/context.rs`  
**Input:** `TileList` + `TextureStore`  
**Output:** `RenderList` populated with `DisplayItem::Blit`

The final stage assembles rasterized tiles into the display list that backends
consume. Only `Clean` tiles (those that were successfully rasterized) are
included.

### Algorithm

```
for tile in tile_list.all_tiles():
    if tile.state == Clean and tile.texture_id is Some:
        texture = texture_store.get(texture_id)
        render_list.push(DisplayItem::Blit {
            x: tile.rect.x,
            y: tile.rect.y,
            w: texture.width,
            h: texture.height,
            data: texture.data.clone(),   // ARgb32, stride = w * 4
        })
```

`Empty` tiles are skipped (transparent region, no need to blit).

### Current limitation

Stages 6 and 7 are currently gated on `#[cfg(feature = "backend_cairo")]`.
The Vello backend has its own rasterization path and does not use the
`TextureStore` blit approach. Supporting multiple compositing strategies is
future work.

---

## Dirty flags and caching

The pipeline short-circuits at `rebuild_render_list_if_needed()` when
`render_dirty` is false. Within the pipeline itself there is currently no
incremental caching — all seven stages run from scratch on every dirty frame.

Tile `TileState` is reset to `Dirty` at the start of each pipeline run.
Future optimisations could preserve `Clean` tiles whose source elements have
not changed.
