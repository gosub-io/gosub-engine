# Layering and compositing

How the pipeline decides which elements get their own layer, and how a layer's group opacity and scroll behaviour are realised at composite time. This expands on [Stage 3 in stages.md](stages.md#stage-3--layering) and the compositing paths in [backends.md](backends.md).

Everything here lives in `crates/gosub_render_pipeline/src/layering/layer.rs` (layer assignment) and `crates/gosub_interface/src/render/backend.rs` (`TileAnchor`, `StickyConstraint`, and the pixel-blend helpers used at composite time).

## Why layers exist

Some CSS effects cannot be baked into a tile's pixels because they depend on state that changes *after* rasterization:

- **`opacity < 1`** on an element fades the element *and its whole subtree* as a single group. Fading each descendant's pixels individually gives a different (wrong) result where children overlap.
- **`position: fixed`** pins an element to the viewport — its screen position changes on every scroll without any pixel changing.
- **`position: sticky`** is scroll-dependent in a more complex way: it scrolls normally, then sticks, then gets shoved off by its container.

The pipeline handles these by *promoting* such elements to their own layer. The layer's tiles are rasterized once, normally; the fade and the scroll-dependent placement are applied every frame by the compositor, which is cheap. Scrolling a page with a translucent sticky header re-blends cached pixels — it never re-rasterizes.

## Promotion rules (`LayerList::traverse`)

The layout tree is walked once from the root. Each element lands either in the current layer or in a newly promoted one. Only the element's **own** (non-inherited) style is inspected — the element that declares `opacity: 0.5` establishes the group; its descendants inherit the effect through the layer and must not re-promote.

An element is promoted, taking its whole subtree with it, when:

| Trigger | Kind | Promotes even inside another promoted group? |
|---|---|---|
| `opacity < 1` | compositing (fade as a group) | yes |
| `position: fixed` | compositing (viewport-pinned) | yes |
| `position: sticky` | compositing (scroll-dependent offset) | yes |
| explicit `z-index` on a positioned element | re-levelling only | no — promotes once at the top of a group |

The distinction in the last column: a *compositing* reason must survive nesting — a faded `<img>` inside a `z-index` container still needs its own faded layer, or the fade would be swallowed by the parent layer. A plain `z-index` only changes *where in the stack* its subtree composites, so once inside a promoted group it just passes its stacking level down instead of splitting off another layer.

Two more assignment rules:

- **Standalone images** (`<img>` outside any promoted group) get their own layer at their stacking level, preserving the older images-on-top behaviour. Inside a promoted group an image deliberately stays in the group layer, so it moves and fades with the group.
- **Everything else** joins the enclosing layer.

### Stacking order

Each layer records an `order` (an `isize`): the promoting element's `z-index` if it has one, otherwise the enclosing stacking context's level (`inherited_order`, 0 at the root). Negative values composite behind the base layer. After traversal `layer_ids` is sorted by `order` — the sort is **stable**, so layers at the same level keep DOM/creation order, which is the correct tie-break for equal `z-index`. The compositor and hit-testing both walk layers in this order.

`z-index` is honoured only on positioned elements (`relative` / `absolute` / `fixed` / `sticky`), matching CSS.

## Group opacity

A layer promoted for `opacity < 1` carries `Layer::opacity`. Its elements are painted and rasterized **at full opacity**; the compositor fades the layer's tiles as a unit by scaling each premultiplied source pixel with `scale_premul_argb_u32(pixel, opacity)` before the source-over blend. Scaling all four channels by the same factor keeps the pixel premultiplied — this is exactly CSS group opacity.

### Avoiding double-darkening

The painter also applies per-element opacity when generating brushes. For elements inside an opacity group that would fade them twice: once by the painter, once by the compositor. The layering stage therefore records the affected DOM nodes, and the painter checks `LayerList::is_opacity_grouped(node_id)` and skips per-element opacity for:

- the promoting element itself (its declared opacity is what the layer fade realises), and
- descendants that declare **no** opacity of their own (they rely entirely on the layer fade).

A descendant that declares its *own* `opacity` inside a faded group keeps applying it per-element, stacking with the group fade. That is an approximation: correct nesting would require a nested compositing group (see limitations).

## Scroll anchors (`TileAnchor`)

Every layer carries a `TileAnchor` describing how its tiles respond to scroll at composite time:

```rust
pub enum TileAnchor {
    Scroll,                    // normal flow: composited at page - scroll
    Fixed,                     // position: fixed — ignores scroll, pinned to viewport
    Sticky(StickyConstraint),  // scrolls, then sticks, then is shoved off by its container
}
```

`anchored_tile_pos(page_x, page_y, scroll_x, scroll_y, anchor)` converts a tile's page-space position to viewport coordinates:

| Anchor | Viewport position |
|---|---|
| `Scroll` | `page - scroll` |
| `Fixed` | `page` (page position equals viewport position) |
| `Sticky(c)` | `page - scroll + c.offset(scroll)` |

Anchors compose with opacity: a translucent fixed navbar is one layer with `opacity < 1` **and** `anchor = Fixed`.

### Sticky: the three regimes

A sticky element lays out in normal flow (like `relative`); layering captures a `StickyConstraint` holding its natural margin box and its containing block's content box (the *cage*), both in page space. At composite time `StickyConstraint::offset(scroll)` returns a translation applied uniformly to every tile in the layer, so the layer moves as a rigid unit. For the vertical axis:

```text
want  = max(0, top − (natural_y − scroll_y))    // push needed to rest at the inset
slack = max(0, cage_bottom − natural_bottom)     // room before hitting the cage edge
dy    = min(want, slack)
```

The clamp produces all three regimes. Worked example — `top: 10px`, natural top at page `y = 100`, element height 50, cage ending at `y = 400` (so `slack = 400 − 150 = 250`):

| `scroll_y` | `want = scroll − 90` | `dy` | Regime |
|---|---|---|---|
| 0 | 0 | 0 | flowing — scrolls normally |
| 150 | 60 | 60 | stuck — top rests at 10 px from viewport top |
| 340 | 250 | 250 | at the cage edge |
| 400 | 310 | 250 (clamped) | shoved — pinned to the cage bottom, scrolls away with it |

Insets are `Option`s: an `auto` edge never sticks. Only `top` and `left` are implemented — `bottom`/`right` need the viewport extent, which the constraint doesn't carry yet. The cage is currently approximated by the parent's content box, and the scrollport is always the viewport (no sub-scroll-containers yet). `StickyConstraint::offset` is unit-tested in `backend.rs`.

## From layers to composited pixels

The layer metadata has to survive the tiling and caching stages to reach the compositor:

1. **Tiling** builds a *separate tile grid per layer* (`TileList.tiles: HashMap<LayerId, TileLayer>`). A sticky header and the base content can therefore both own a tile at the same page position.
2. **The engine's tile cache** (`crates/gosub_engine/src/engine/context.rs`) keys rasterized tiles by `(page_x, page_y, layer_id, content_hash)` — `layer_id` disambiguates same-position tiles from different layers.
3. **Tile transport** stamps each tile with its layer's `opacity` and `anchor`: `CachedTile` (CPU pixels) and `PlacedGpuTile` (GPU texture id) both carry the pair, so compositors need no access to the `LayerList`.
4. **Compositors** — the host examples' CPU blitters and the shared wgpu tile compositor (`gosub_renderer_vello/src/gpu_tiles.rs`) — walk tiles in layer order and, per tile: place it with `anchored_tile_pos`, scale by `scale_premul_argb_u32` when `opacity < 1`, and blend with the source-over operator `blend_over_argb_u32`. `CachedTile.opaque` (computed once when caching) lets CPU compositors skip the per-pixel blend for fully opaque tiles and do a plain row copy.

### The GPU one-shot scene path

GPU backends that render the whole viewport as a single scene (no tiles) get the same semantics through paint commands: `Painter::paint_all` wraps each promoted layer's commands in `PaintCommand::PushLayer { opacity, anchor } … PopLayer`, and the backend translates that into its native compositing group (e.g. a Vello layer). The base scroll layer at full opacity gets no wrapper. See [gpu-render-flow.md](gpu-render-flow.md) for how this path relates to the tile path.

Note that per-tile rasterizers never see `PushLayer`/`PopLayer` — the tile path applies opacity and anchoring at composite time, so tile rasterizers simply ignore those commands.

## Hit-testing across layers

`LayerList::find_element_at(vp_x, vp_y, scroll_x, scroll_y)` walks layers **top-to-bottom** (reverse `layer_ids` order) and inverts each layer's composite mapping to convert the viewport point into that layer's page space: fixed layers are tested at the raw viewport coordinate, scrolling layers at `viewport + scroll`, sticky layers at `viewport + scroll − sticky_offset`. This is why hovering a fixed navbar works regardless of scroll position.

## Current limitations

- **Nested opacity** inside a faded group stacks per-element instead of forming a nested compositing group.
- **Sticky `bottom`/`right`** insets and **percentage/em insets** are not resolved; the sticky cage is the parent's content box, not the true containing block; no sub-scroll-containers.
- **`mix-blend-mode`, transforms, filters** do not promote or composite yet.
- **Hit-testing** scans element boxes linearly per layer (an R-tree is planned; the tiler already uses one for tiles).
