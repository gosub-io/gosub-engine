# Data Structures

This document describes the key types that flow through the render pipeline.
Types are listed in pipeline order.

---

## ID types

All pipeline IDs are newtypes over `u64` and compare equal when the underlying
value matches. Conversions between related ID types (e.g. `RenderNodeId` ↔
`NodeId`) are provided by `From`/`Into` impls.

| Type | Crate | Meaning |
|------|-------|---------|
| `NodeId` | `gosub_shared` | DOM node in the parsed document |
| `RenderNodeId` | `gosub_render_pipeline` | Node in the render tree (filtered DOM) |
| `LayoutElementId` | `gosub_render_pipeline` | Node in the layout tree |
| `LayerId` | `gosub_render_pipeline` | Rendering layer |
| `TileId` | `gosub_render_pipeline` | Individual tile |
| `TextureId` | `gosub_render_pipeline` | Rasterized pixel buffer |
| `MediaId` | `gosub_render_pipeline` | Image or SVG resource |

---

## Document adapter — `PipelineDocument`

**File:** `crates/gosub_render_pipeline/src/common/document/pipeline_doc.rs`

```rust
pub trait PipelineDocument: Send + Sync {
    fn root(&self) -> Option<NodeId>;
    fn children(&self, id: NodeId) -> Vec<NodeId>;
    fn node_kind(&self, id: NodeId) -> PipelineNodeKind;
    fn tag_name(&self, id: NodeId) -> Option<String>;
    fn is_display_none(&self, id: NodeId) -> bool;
    fn parent(&self, id: NodeId) -> Option<NodeId>;
    /// Own (non-inherited) value — the trait's style primitive.
    fn get_own_style(&self, id: NodeId, prop: &StyleProperty) -> Option<Value>;
    // Provided methods layered on top of get_own_style:
    fn get_style(&self, id: NodeId, prop: &StyleProperty) -> Value;      // + inheritance, initial values, em/rem→px
    fn get_style_f32(&self, id: NodeId, prop: &StyleProperty) -> f32;
    fn html_node_id(&self) -> Option<NodeId>;
    fn body_node_id(&self) -> Option<NodeId>;
    fn base_url(&self) -> String;
    fn get_node_by_id(&self, id: NodeId) -> Option<Node>;
    fn inner_html(&self, id: NodeId) -> String;
}
```

The concrete implementation for the Gosub DOM is `GosubDocumentAdapter<C>`,
which wraps `Arc<C::Document>` and lazily computes CSS properties via
`C::CssSystem::properties_from_node()`. Computed styles **are cached** per node
inside the adapter (separate caches for computed styles, inline `style=""`
attributes, and `::before`/`::after` pseudo-boxes); `clear_style_cache()` /
`invalidate_style_for_nodes()` evict entries for `:hover` re-matching. See
[The two worlds](../two-worlds.md).

The `root()` implementation returns the `<html>` element (not the synthetic
`Document` node at index 0) because the render pipeline expects a real element
as its root. When the document root is already an element (fragment documents),
it falls back to `doc.root()` directly.

---

## Stage 1 output — `RenderTree`

**File:** `crates/gosub_render_pipeline/src/rendertree_builder/tree.rs`

```rust
pub struct RenderTree {
    pub doc: Arc<dyn PipelineDocument>,
    pub arena: HashMap<RenderNodeId, RenderNode>,
    pub root_id: Option<RenderNodeId>,
}

pub struct RenderNode {
    pub node_id: RenderNodeId,
    pub children: Vec<RenderNodeId>,
}
```

The render tree is a lightweight mirror of the visible DOM. It holds no style
data itself — style queries go through `doc.get_style()` on demand.

---

## Stage 2 output — `LayoutTree`

**File:** `crates/gosub_render_pipeline/src/layouter.rs`

```rust
pub struct LayoutTree {
    pub render_tree: RenderTree,
    pub arena: HashMap<LayoutElementId, LayoutElementNode>,
    pub root_id: LayoutElementId,
    pub root_dimension: Dimension,
}

pub struct LayoutElementNode {
    pub id: LayoutElementId,
    pub dom_node_id: DomNodeId,
    pub render_node_id: RenderNodeId,
    pub parent: Option<LayoutElementId>,   // containing-block walk (e.g. sticky cage)
    pub children: Vec<LayoutElementId>,
    pub box_model: BoxModel,
    pub context: ElementContext,
    pub background_media: Option<BackgroundMedia>, // resolved CSS background-image
}
```

### `BoxModel`

```rust
pub struct BoxModel {
    pub margin_box:  Rect,   // outer edge
    pub border_box:  Rect,   // inside margin
    pub padding_box: Rect,   // inside border
    pub content_box: Rect,   // inside padding
}
```

All four boxes share the same origin offset. `content_box` is used for text
and image intrinsic sizing.

### `ElementContext`

Carries metadata that is only meaningful for specific element kinds:

```rust
pub enum ElementContext {
    None,
    Text(ElementContextText),
    Image(ElementContextImage),
    Svg(ElementContextSvg),
}
```

`ElementContextText` holds the string value, `FontInfo`, and a `text_offset`
(where the baseline sits relative to the content box). `ElementContextImage`
and `ElementContextSvg` hold the `MediaId` and the intrinsic `Dimension`.

---

## Stage 3 output — `LayerList`

**File:** `crates/gosub_render_pipeline/src/layering/layer.rs`

```rust
pub struct LayerList {
    pub layout_tree: Arc<LayoutTree>,
    layer_ids: RwLock<Vec<LayerId>>,
    layers: RwLock<HashMap<LayerId, Layer>>,
    opacity_group_nodes: RwLock<HashSet<NodeId>>, // nodes whose paint skips per-element opacity
}

pub struct Layer {
    pub layer_id: LayerId,
    pub order: isize,        // compositing z-order (from z-index); higher = on top
    pub opacity: f32,        // group opacity, applied at composite time
    pub anchor: TileAnchor,  // Scroll / Fixed / Sticky — scroll behaviour at composite time
    pub elements: Vec<LayoutElementId>,
}
```

Iteration order for compositing: layers sorted ascending by `order` (stable, so equal
orders keep creation order). See [layering-and-compositing.md](layering-and-compositing.md)
for how layers are assigned and how `opacity`/`anchor` are realised.

---

## Stage 4 output — `TileList`

**File:** `crates/gosub_render_pipeline/src/tiler.rs`

```rust
pub struct TileList {
    pub layer_list: Arc<LayerList>,
    pub tiles: HashMap<LayerId, TileLayer>,
    pub arena: HashMap<TileId, Tile>,
    pub default_tile_dimension: Dimension,
}

pub struct Tile {
    pub id: TileId,
    pub layer_id: LayerId,
    pub elements: Vec<TiledLayoutElement>,
    pub texture_id: Option<TextureId>,
    pub state: TileState,
    pub rect: Rect,           // position in page space
    pub bgcolor: Option<(f32,f32,f32,f32)>,
}

pub struct TiledLayoutElement {
    pub id: LayoutElementId,
    pub rect: Rect,           // element rect clipped to tile boundary
    pub position: Coordinate, // element origin within tile-local space
    pub paint_commands: Vec<PaintCommand>,
}

pub enum TileState {
    Dirty,           // needs rasterization
    Clean,           // has valid texture
    Empty,           // no visible content
    Unrenderable,    // backend cannot handle
}
```

`TileLayer` wraps an `rstar::RTree` of rectangle primitives tagged with their
`TileId` (`GeomWithData<Rectangle<[f64; 2]>, TileId>`) for spatial queries.

---

## Stage 5 output — `PaintCommand`

**File:** `crates/gosub_render_pipeline/src/painter/commands.rs`

```rust
pub enum PaintCommand {
    Text(Text),
    Rectangle(Rectangle),
    Svg(PaintSvg),
}
```

### `Rectangle`

```rust
pub struct Rectangle {           // fields private; built via with_* builder methods
    rect:          Rect,
    background:    Option<Brush>,
    border:        Border,
    radius_top:    Radius,       // one Radius per corner
    radius_right:  Radius,
    radius_bottom: Radius,
    radius_left:   Radius,
}
```

`Border` carries per-side arrays: `widths: [f32; 4]`, `styles: [BorderStyle; 4]`,
and `brushes: [Brush; 4]` (per-side colors are brushes, not a single `Color`).
`Radius { x: f64, y: f64 }` is one corner's ellipse radii; the four corners live
as separate fields on `Rectangle`.

### `Brush`

```rust
pub enum Brush {
    Solid(Color),
    Image(MediaId),
    Gradient(Gradient),
}
```

### `Text`

```rust
pub struct Text {
    pub text:      String,
    pub rect:      Rect,
    pub font_info: FontInfo,
    pub brush:     Brush,
}
```

`FontInfo` carries `family: String`, `size: f64`, `weight: i32`, plus style
(slant), line height, alignment, and decoration flags.

---

## Stage 6 output — `TextureStore`

**File:** `crates/gosub_render_pipeline/src/common/texture_store.rs`

```rust
pub struct TextureStore {
    textures: HashMap<TextureId, Arc<Texture>>,
    next_id:  TextureId,
}

pub struct Texture {
    pub id:     TextureId,
    pub width:  usize,
    pub height: usize,
    pub pixels: TilePixels,     // Cpu(Arc<Vec<u8>>) or Gpu(u64 texture id)
    pub format: PixelFormat,    // self-describing byte order (see backend.rs)
}
```

CPU pixels are reference-counted (`Arc`) so the compositing stage can hand the
data directly into `DisplayItem::Blit` without a copy when possible; GPU tiles
carry a backend-owned texture id instead.

---

## Stage 7 output — `RenderList`

**File:** `crates/gosub_interface/src/render/render_list.rs` (re-exported via `gosub_render_pipeline::render`)

```rust
pub struct RenderList {
    pub items: Vec<DisplayItem>,
}

pub enum DisplayItem {
    Clear  { color: Color },
    Rect   { x: f32, y: f32, w: f32, h: f32, color: Color },
    TextRun { x: f32, y: f32, text: String, size: f32, color: Color, max_width: Option<f32> },
    Blit   { x: f32, y: f32, w: u32, h: u32, data: Arc<Vec<u8>> },
}
```

Only `Clear` and `Blit` are used by the current pipeline:

- `Clear` — written by the pipeline as the first item to fill the background
  white before any tiles are blitted.
- `Blit` — one per clean tile; coordinates are in **page space** (the backend
  applies a viewport-offset transform before drawing).

`Rect` and `TextRun` are fallback variants used by the non-pipeline rendering
path and available to backends that bypass the tile compositor.

---

## Supporting stores

### `MediaStore`

**File:** `crates/gosub_render_pipeline/src/common/media.rs`

Holds decoded images and parsed SVG documents keyed by `MediaId`. Loaded on
demand during layout (for intrinsic sizing) and rasterization. A single
`MediaStore` instance is created per pipeline run and passed through Stages 6
and 7.

> **Note:** `TaffyLayouter` creates its own `MediaStore` by default, but the
> engine shares one store into it via `set_media_store()` so resources loaded
> during layout are visible to the rasterizer. Forgetting to share the store
> (in a custom integration) makes images render as placeholders — see
> [layout.md](layout.md).

### `BrowserState`

**File:** `crates/gosub_render_pipeline/src/common/browser_state.rs`

Carries the current viewport rect and DPI scale factor. Passed to the painter
so that viewport-relative units (e.g. `vw`, `vh`) can be resolved.
