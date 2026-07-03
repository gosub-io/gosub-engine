# `gosub_interface`: the trait families

`gosub_interface` is the dependency-inversion heart of the workspace. It defines the trait
contracts for every major engine component; component crates (`gosub_html5`, `gosub_css3`,
the renderer crates, …) depend on the interface and implement its traits, and never on each
other. Generic engine code names components only through a config type.

This page is the crate-side architecture view. For the *user's* view — how to pick
components when embedding the engine — see [configuration.md](configuration.md).

## The configuration model

`ModuleConfiguration` (`config.rs`) is a compile-time description of a component set: a
client defines one zero-sized marker type and names each component as an associated type.

```rust
pub trait ModuleConfiguration: /* … */ {
    type CssSystem: CssSystem;
    type Document: Document<Self>;
    type HtmlParser: Html5Parser<Self>;
}
```

Three properties define the model:

- **Compile-time resolution.** The engine is generic over `C: ModuleConfiguration`; there
  is no runtime registry. Naming a component implies a Cargo dependency on the crate that
  provides it — you cannot name `CairoBackend` without compiling Cairo in.
- **Narrow `Has*` view traits.** Subsystem code doesn't bound on the full config; it asks
  for exactly what it needs (`HasCssSystem`, `HasDocument`, `HasHtmlParser`, `HasLayouter`,
  `HasFontSystem`). These are **derived automatically** from a `ModuleConfiguration` via
  blanket impls — never implemented by hand. A function bounded on `C: HasDocument`
  therefore accepts any config, but can only touch the document and CSS system.
- **Parse-only by design.** `ModuleConfiguration` deliberately contains no render types.
  The rendering extension, `RenderConfiguration` (adding `RenderBackend`,
  `CompositorSink`, `FontSystem`), lives in **`gosub_engine`** (`src/html.rs`), not here —
  so parse-only configs (parser test harnesses, fuzz targets) never pull in renderer
  crates. `DefaultRenderConfig<Backend, FontSystem, Compositor>` is the ready-made
  implementation.

## The trait families

### Document (`document.rs`, `node.rs`)

`Document<C>` is a storage-agnostic DOM: all access goes through `NodeId` handles and the
document answers questions about nodes — no `&Node` is ever handed out, so the concrete
storage (arena, column store, …) stays hidden. It covers node creation, tree surgery
(`attach`/`detach`/`relocate_node`), element data (attributes, classes, `<template>`
contents), quirks mode, and serialisation. Stylesheets attach to the document
(`add_stylesheet`/`stylesheets`), typed via the config's CSS system. Implemented by
`gosub_html5::DocumentImpl`.

### HTML parsing (`html5.rs`)

`Html5Parser<C>` is two functions — `parse` and `parse_fragment` — from a `ByteStream`
into a `C::Document`, returning recoverable parse errors. Implemented by `gosub_html5`.

### CSS (`css3.rs`)

`CssSystem` bundles four associated types (`Stylesheet`, `PropertyMap`, `Property`,
`Value`) and the operations other crates need without understanding CSS internals:

- `parse_str` — text → stylesheet, tagged with a `CssOrigin` (UserAgent / Author / User);
- `properties_from_node` — selector matching + cascade for one node (`None` = not
  renderable), and `pseudo_properties_from_node` for `::before`/`::after`;
- `load_default_useragent_stylesheet`;
- `hover_fingerprints` — scans stylesheets for the element types/classes/ids targeted by
  `:hover` rules, so the engine can skip style recalculation for pointer moves that no
  hover rule could affect. It lives on this trait because only the CSS implementation
  understands its own selector representation.

`CssProperty`/`CssValue` are deliberately lowest-common-denominator accessor traits
(`as_string`, `as_unit`, `as_color`, `as_list`, …) — consumers like the pipeline's
[document adapter](two-worlds.md) probe values through these and convert into their own
representation. Implemented by `gosub_css3::Css3`.

### Layout (`layout.rs`)

The interface-world layout family: `Layouter<C>`, `LayoutTree<C>` (with its own node-id
space), `LayoutNode`, `LayoutCache`, `Layout` (the per-node geometry), and
`TextLayout`/`HasTextLayout` for shaped text. Implemented by `gosub_taffy` — which is
currently **dormant**: the live rendering path uses the pipeline's own layouter instead.
See [The two worlds](two-worlds.md) before spending time here.

### Fonts (`font.rs`, `font_system.rs`)

`FontSystem` (register + measure, never draw) with its value types (`FontBlob`,
`FontStyle`, `TextStyle`, `ShapedText`, …) and the `HasFontSystem` view. Covered in depth
in [fonts.md](fonts.md).

### Render (`render/`)

The contract between the render pipeline and the concrete backends. It lives *here* — not
in `gosub_render_pipeline` — so a config can name a `RenderBackend` without inverting the
dependency direction; the pipeline re-exports these types for downstream code.

- `RenderBackend` — surface creation, `render`, `snapshot`, `external_handle`, plus the
  capability flags the engine uses to pick a flow without knowing which backend is active:
  `raster_strategy()` (ParallelCached / Sequential / None), `renders_to_gpu_texture()`,
  `gpu_tile_compositing()`, `device_pixel_ratio()`, and `composite_tiles()` for GPU tile
  blitting. See [render-pipeline/backends.md](render-pipeline/backends.md) and
  [gpu-render-flow.md](render-pipeline/gpu-render-flow.md) for how these drive the flows.
- `ErasedSurface`, `RenderContext` (per-tab state a backend needs: viewport, `RenderList`,
  scroll offset, type-erased paint scene), `CompositorSink` (receives finished frames per
  tab), `ExternalHandle` (the many ways a frame can travel: CPU pixels, tile cache, GL/wgpu
  texture ids, …).
- Value types shared by every compositor: `PixelFormat` (self-describing byte order) with
  the hot blend helpers (`blend_over_argb_u32`, `scale_premul_argb_u32`), `TileAnchor` /
  `StickyConstraint`, `CachedTile` / `PlacedGpuTile`, `Viewport` / `DevicePixelRatio`, and
  `RenderList` / `DisplayItem`. The compositing semantics are documented in
  [layering-and-compositing.md](render-pipeline/layering-and-compositing.md).

### Input (`input.rs`)

`InputEvent` / `MouseButton` — the small event vocabulary UAs feed into the engine
(consumed by `gosub_web_platform` and tab input handling).

## The type-erasure escape hatches

The interface crate sits at the bottom of the dependency graph, so it can never name types
from the crates above it. Where a contract genuinely needs to carry such a type, the crate
uses a deliberate, documented `Any` seam — the owning crate downcasts on the other side:

| Seam | Carries | Downcast by |
|---|---|---|
| `RenderBackend::create_rasterizer → Box<dyn Any>` | a `Box<dyn Rasterable>` (pipeline tile/texture types) | `gosub_render_pipeline::rasterizer::downcast_rasterizer` |
| `RenderBackend::wgpu_resources → Arc<dyn Any>` | a backend's shared GPU device/queue/renderer | the code that shares the wgpu context |
| `RenderContext::paint_scene → &dyn Any` | the pipeline's `PaintScene` for GPU scene backends | the GPU backend's `render` |
| `FontSystem::as_any_mut → &mut dyn Any` | the concrete font system | a render backend's native shape/draw path |
| `ErasedSurface::as_any(_mut)` | a backend's concrete surface | the backend itself |

The pattern is consistent: *traits define behaviour the engine drives; `Any` carries state
that only one crate on each side understands.* If you find yourself wanting to add a
pipeline or backend type to a trait in this crate, one of these seams (or a new one) is
the intended alternative.
