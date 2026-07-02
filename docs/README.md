# Gosub engine documentation

An index of everything under `/docs`. Pages marked **planned** don't exist yet; the
descriptions say what they should cover so we can decide what to write next.

## Getting started

- [Tutorial](tutorial.md) — engine / zone / tab concepts and a step-by-step first integration.
- [Configuration](configuration.md) — choosing your components: render backend, font system,
  `DefaultRenderConfig`, and going fully custom.
- [Running the examples](examples.md) — headless engine examples and the GUI toolkit examples
  (winit, GTK4, egui).
- [Development](development.md) — running tests and benchmarks.
- [WebAssembly](webassembly.md) — building for wasm.
- [Component tools](binaries.md) — the small per-crate CLI tools (`css3-parser`,
  `html5-parser-test`, `run-js`, …).

## Architecture

- [Crates overview](crates.md) — one section per workspace crate and how they depend on
  each other. Start here to find where something lives.
- [Fonts](fonts.md) — the two font backend families: *font systems* (measure text for
  layout) vs *text rasterizers* (draw glyphs), and how one shared font collection keeps
  them consistent.
- [Render pipeline](render-pipeline/README.md) — the rasterize-and-composite pipeline:
  - [Stages](render-pipeline/stages.md) — render tree → layout → paint → tile → rasterize → composite.
  - [Data structures](render-pipeline/data-structures.md)
  - [Backends](render-pipeline/backends.md) — Cairo, Skia (CPU/GPU), Vello, and the dynamic backend.
  - [GPU render flow](render-pipeline/gpu-render-flow.md)
- [Cookies](cookies.md) — the cookie subsystem inside `gosub_engine`.
- [Storage](datastores.md) — localStorage / sessionStorage architecture.

## Networking

The networking stack (`gosub_net` and the docs under [network/](network/)) is moving to its
own project, **gosub_sonar**, which carries its own documentation. The pages here
([net-architecture](network/net-architecture.md), [net-design](network/net-design.md),
[pump](network/pump.md)) describe the in-tree crate until the move completes.

## Planned

Pages we still want to write, roughly in priority order:

- **The two worlds** — `gosub_render_pipeline` has its own document/style/layout types and
  its own Taffy layouter, parallel to the `gosub_html5`/`gosub_css3`/`gosub_taffy` world
  behind `gosub_interface`. Both bridge tables to `gosub_lattice`. Explain the split and
  where the seam is.
- **`gosub_interface` trait families** — the dependency-inversion crate:
  `ModuleConfiguration` and the `Has*` view traits, `CssSystem`, `Document`, `Layouter`,
  `FontSystem`, `RenderBackend`, and the deliberate type-erasure escape hatches
  (`create_rasterizer → Box<dyn Any>`, `FontSystem::as_any_mut`).
- **Tiling & compositing** (extend `render-pipeline/`) — self-describing pixel formats
  (`PreMulArgb32` vs `Rgba8` and why), `TileAnchor` scroll/fixed/sticky compositing,
  layer promotion and group opacity, `RasterStrategy`.
- **CSS system internals** — parse → match → cascade → computed values, and the formal
  property-value grammar validator (`syntax_matcher`).
- **Layout** — Taffy integration (`LayoutDocument` implements Taffy's traits over the real
  DOM), inline layout, and `gosub_lattice` table layout with its two-pass write-back.
- **HTML5 parsing** — tokenizer / tree-builder structure, the html5lib test harness, and
  the current limitation that parsing is not incremental.
- **JavaScript stack** — `gosub_webexecutor` (abstraction) → `gosub_v8` (bindings) →
  `gosub_webinterop` (proc-macro glue) → `gosub_jsapi` (web APIs), plus
  `gosub_web_platform` (event loop, timers).
- **Headless usage** — `bin/gosub-screenshot` as the reference for driving the engine
  without a window (tile-cache compositing, no GPU texture-size limit).
