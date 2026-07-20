# gosub_render_pipeline

The render pipeline of the Gosub browser engine: DOM + CSSOM in, rasterized tiles out. It
is a staged, tile-based pipeline — rasterized tiles are cached across frames, and a
change re-runs only the stages it invalidates, so a pure scroll costs a re-composite
instead of a re-render. The render backends (`gosub_renderer_cairo` / `_skia` / `_vello`)
implement against the contracts surfaced here.

The pipeline deliberately uses its own document/style/layout types, separate from the
`gosub_interface` world where parsing happens; the adapter that joins the two sides is
described in [docs/two-worlds.md](../../docs/two-worlds.md).

## The stages

| # | Stage | Module | What it does |
|---|-------|--------|--------------|
| 1 | Render tree | `rendertree_builder` | DOM → `RenderTree`; prunes invisible nodes (`head`, `script`, ...) |
| 2 | Layout | `layouter` | Box models via Taffy, plus the pipeline's own inline-run layout; text measured through the configured `FontSystem`; tables via `gosub_lattice` |
| 3 | Layering | `layering` | Promotes `opacity` / `fixed` / `sticky` / `z-index` elements into layers → `LayerList` |
| 4 | Tiling | `tiler` | Subdivides the page into tiles (256×256 by default) → `TileList` |
| 5 | Painting | `painter` | Generates backend-agnostic `PaintCommand`s; glyph runs are pre-shaped here so backends only draw |
| 6 | Rasterization | `rasterizer` | `Rasterable` backends execute the commands per tile → `BakedTile` pixel data |
| 7 | Compositing | (engine + host) | Per-frame: visible cached tiles are composited with the scroll offset applied |

Stages 1–6 are driven by `gosub_engine`'s `BrowsingContext` (`pipeline_build_cache`) on
content change; stage 7 runs every frame. Which path a tab takes — CPU tile cache
(Cairo/Skia) or GPU scene (Vello) — is decided by the backend's capability queries
(`raster_strategy()`, `renders_to_gpu_texture()`).

## The backend contract

Backends implement two things:

- `RenderBackend` (from `gosub_interface`, re-exported via `render::backend`) — surfaces,
  `render()`, capability queries, `ExternalHandle` output.
- `Rasterable` (this crate, `rasterizer`) — per-tile execution of paint commands, with a
  `RasterStrategy` (parallel-cached for CPU backends, sequential for Vello).

`common` holds what the stages share: the pipeline's document/style types, geometry, the
media store (decoded images/SVG) and the texture store. Spatial queries (visible-element
and hit tests) go through rstar R-trees.

## Features

`wayland` / `x11` — GDK platform integration (Linux only).

## Further reading

- [docs/render-pipeline/README.md](../../docs/render-pipeline/README.md) — the full
  pipeline documentation: two-phase design, entry points, fast paths
- [docs/render-pipeline/stages.md](../../docs/render-pipeline/stages.md) — each stage in
  depth
- [docs/render-pipeline/data-structures.md](../../docs/render-pipeline/data-structures.md)
  — the types flowing between stages
- [docs/render-pipeline/layout.md](../../docs/render-pipeline/layout.md) — stage 2:
  Taffy, inline emulation, tables
- [docs/render-pipeline/layering-and-compositing.md](../../docs/render-pipeline/layering-and-compositing.md)
  — layer promotion and the compositor
- [docs/render-pipeline/backends.md](../../docs/render-pipeline/backends.md) — the
  backend contract and host compositing
- [docs/two-worlds.md](../../docs/two-worlds.md) — why the pipeline has its own types
