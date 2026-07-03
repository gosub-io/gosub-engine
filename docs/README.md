# Gosub engine documentation

An index of everything under `/docs`. Pages marked **planned** don't exist yet; the
descriptions say what they should cover so we can decide what to write next.

## Getting started

- [Tutorial](tutorial.md) — engine / zone / tab concepts and a step-by-step first integration.
- [Configuration](configuration.md) — choosing your components: render backend, font system,
  `DefaultRenderConfig`, and going fully custom.
- [Running the examples](examples.md) — headless engine examples and the GUI toolkit examples
  (winit, GTK4, egui).
- [Headless usage](headless.md) — rendering real pages without a window, with
  `gosub-screenshot` as the worked reference (CPU Skia, tile-cache compositing).
- [Development](development.md) — running tests and benchmarks.
- [WebAssembly](webassembly.md) — building for wasm.
- [Component tools](binaries.md) — the small per-crate CLI tools (`css3-parser`,
  `html5-parser-test`, `run-js`, …).

## Architecture

- [Crates overview](crates.md) — one section per workspace crate and how they depend on
  each other. Start here to find where something lives.
- [The two worlds](two-worlds.md) — why there are two parallel document/style/layout
  stacks (the `gosub_interface` world and the pipeline's own types), and the
  `GosubDocumentAdapter` seam that joins them. Read this before diving into either.
- [Interface trait families](interface.md) — `gosub_interface` as the dependency-inversion
  crate: `ModuleConfiguration` and the `Has*` view traits, the per-component contracts,
  and the deliberate type-erasure escape hatches.
- [CSS internals](css.md) — `gosub_css3` from text to computed value: parsing, selector
  matching, the value-grammar validator, shorthand expansion, and the cascade.
- [HTML5 parsing](html5.md) — `gosub_html5`: the spec tokenizer state machine, the
  insertion-mode tree builder, the arena DOM, and the html5lib test harness.
- [Lattice table layout](lattice.md) — `gosub_lattice`: the CSS table algorithm that works
  in conjunction with Taffy via the `TableTree` adapter (grid geometry by lattice, cell
  contents by the host engine).
- [JavaScript stack](javascript.md) — the five scripting crates (`webexecutor` abstraction,
  V8 bindings, proc-macro glue, web APIs, event loop) and their built-but-not-wired status.
- [Fonts](fonts.md) — the two font backend families: *font systems* (measure text for
  layout) vs *text rasterizers* (draw glyphs), and how one shared font collection keeps
  them consistent.
- [Render pipeline](render-pipeline/README.md) — the rasterize-and-composite pipeline:
  - [Stages](render-pipeline/stages.md) — render tree → layout → paint → tile → rasterize → composite.
  - [Data structures](render-pipeline/data-structures.md)
  - [Layout](render-pipeline/layout.md) — Stage 2 in depth: Taffy integration, the
    anonymous-flex inline emulation, text measurement, and table layout via `gosub_lattice`.
  - [Layering & compositing](render-pipeline/layering-and-compositing.md) — layer promotion
    (opacity, fixed, sticky, z-index), scroll anchors, and group opacity at composite time.
  - [Backends](render-pipeline/backends.md) — Cairo, Skia (CPU/GPU), Vello, and the dynamic backend.
  - [GPU render flow](render-pipeline/gpu-render-flow.md)
- [Zones and tabs](zones-and-tabs.md) — the engine's runtime model: zones as isolated
  profiles, tabs as independent worker tasks, and the command/event flow between them.
- [Resource pipelines](resource-pipeline.md) — how fetched bytes become typed assets
  (HTML/CSS/JS/images/fonts), including parser-driven sub-resource discovery and
  hierarchical fetch cancellation.
- [Cookies](cookies.md) — the cookie subsystem inside `gosub_engine`.
- [Storage](datastores.md) — localStorage / sessionStorage architecture.

## Networking

The networking stack (`gosub_net` and the docs under [network/](network/)) is moving to its
own project, **gosub_sonar**, which carries its own documentation. The pages here
([net-architecture](network/net-architecture.md), [net-design](network/net-design.md),
[pump](network/pump.md)) describe the in-tree crate until the move completes.

## Planned

Nothing at the moment — all originally planned pages exist. Candidates for later:
`EngineSettings` (a section in [configuration.md](configuration.md)), and networking docs
once the **gosub_sonar** move completes.
