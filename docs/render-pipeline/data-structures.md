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
| `RenderNodeId` | `gosub_pipeline` | Node in the render tree (filtered DOM) |
| `LayoutElementId` | `gosub_pipeline` | Node in the layout tree |
| `LayerId` | `gosub_pipeline` | Rendering layer |
| `TileId` | `gosub_pipeline` | Individual tile |
| `TextureId` | `gosub_pipeline` | Rasterized pixel buffer |
| `MediaId` | `gosub_pipeline` | Image or SVG resource |

---

## Document adapter — `PipelineDocument`

**File:** `crates/gosub_pipeline/src/common/document/pipeline_doc.rs`

```rust
pub trait PipelineDocument: Send + Sync {
    fn root(&self) -> Option<NodeId>;
    fn children(&self, id: NodeId) -> Vec<NodeId>;
    fn node_kind(&self, id: NodeId) -> PipelineNodeKind;
    fn tag_name(&self, id: NodeId) -> Option<String>;
    fn is_display_none(&self, id: NodeId) -> bool;
    fn parent(&self, id: NodeId) -> Option<NodeId>;
    fn get_style(&self, id: NodeId, prop: StyleProperty) -> Option<StyleValue>;
    fn get_style_f32(&self, id: NodeId, prop: StyleProperty) -> f32;
    fn html_node_id(&self) -> Option<NodeId>;
    fn body_node_id(&self) -> Option<NodeId>;
    fn base_url(&self) -> String;
    fn get_node_by_id(&self, id: NodeId) -> Option<Node>;
    fn inner_html(&self, id: NodeId) -> String;
}
```

The concrete implementation for the Gosub DOM is `GosubDocumentAdapter<C>`,
which wraps `Arc<C::Document>` and lazily computes CSS properties via
`C::CssSystem::properties_from_node()`. Styles are never cached inside the
adapter — each call recomputes from the stylesheet list.

The `root()` implementation returns the `<html>` element (not the synthetic
`Document` node at index 0) because the render pipeline expects a real element
as its root. When the document root is already an element (fragment documents),
it falls back to `doc.root()` directly.

---

## Stage 1 output — `RenderTree`

**File:** `crates/gosub_pipeline/src/rendertree_builder/tree.rs`

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

**File:** `crates/gosub_pipeline/src/layouter/mod.rs`

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
    pub children: Vec<LayoutElementId>,
    pub box_model: BoxModel,
    pub context: ElementContext,
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

**File:** `crates/gosub_pipeline/src/layering/layer.rs`

```rust
pub struct LayerList {
    pub layout_tree: Arc<LayoutTree>,
    layer_ids: RwLock<Vec<LayerId>>,
    layers: RwLock<HashMap<LayerId, Layer>>,
}

pub struct Layer {
    pub layer_id: LayerId,
    pub order: isize,        // compositing z-order; higher = on top
    pub elements: Vec<LayoutElementId>,
}
```

Iteration order for compositing: layers sorted ascending by `order`.

---

## Stage 4 output — `TileList`

**File:** `crates/gosub_pipeline/src/tiler.rs`

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

`TileLayer` wraps an `rstar::RTree<TileRect>` for spatial queries. Each tile
in the tree stores its bounding rect and `TileId`.

---

## Stage 5 output — `PaintCommand`

**File:** `crates/gosub_pipeline/src/painter/commands.rs`

```rust
pub enum PaintCommand {
    Text(Text),
    Rectangle(Rectangle),
    Svg(PaintSvg),
}
```

### `Rectangle`

```rust
pub struct Rectangle {
    pub rect:   Rect,
    pub brush:  Brush,
    pub border: Option<Border>,
    pub radius: Option<Radius>,
}
```

`Border` has per-side `width: f32`, `style: BorderStyle`, and `color: Color`.
`Radius` has four per-corner values (top-left, top-right, bottom-right,
bottom-left).

### `Brush`

```rust
pub enum Brush {
    Solid(Color),
    Image(MediaId),
    // gradient planned
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

`FontInfo` carries `family: String`, `size: f64`, and `weight: u32`.

---

## Stage 6 output — `TextureStore`

**File:** `crates/gosub_pipeline/src/common/texture_store.rs`

```rust
pub struct TextureStore {
    textures: HashMap<TextureId, Texture>,
}

pub struct Texture {
    pub width:  u32,
    pub height: u32,
    pub data:   Arc<Vec<u8>>,   // premultiplied ARgb32, stride = width * 4
}
```

Textures are reference-counted (`Arc`) so the compositing stage can hand the
data directly into `DisplayItem::Blit` without a copy when possible.

---

## Stage 7 output — `RenderList`

**File:** `crates/gosub_engine/src/render/render_list.rs`

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

**File:** `crates/gosub_pipeline/src/common/media.rs`

Holds decoded images and parsed SVG documents keyed by `MediaId`. Loaded on
demand during layout (for intrinsic sizing) and rasterization. A single
`MediaStore` instance is created per pipeline run and passed through Stages 6
and 7.

> **Note:** `TaffyLayouter` currently creates its own `MediaStore` internally.
> This means images loaded during layout are not shared with the rasterizer's
> store. This is a known limitation.

### `BrowserState`

**File:** `crates/gosub_pipeline/src/common/browser_state.rs`

Carries the current viewport rect and DPI scale factor. Passed to the painter
so that viewport-relative units (e.g. `vw`, `vh`) can be resolved.
