# Configuration: choosing your components

`GosubEngine` is generic over a single *configuration* type that names every pluggable component
at compile time. There is no runtime registry — naming a component (e.g. `CairoBackend`) is what
pulls that crate into your build. This page explains how to pick the right config when you embed
the engine.

## The two layers

The configuration is split into two traits so that tools which never paint don't have to compile
in the renderer crates.

| Trait | Crate | Names | Used by |
|---|---|---|---|
| [`ModuleConfiguration`](../crates/gosub_interface/src/config.rs) | `gosub_interface` | CSS system, DOM document, HTML parser | everyone (parse-only tools implement *only* this) |
| `RenderConfiguration` | `gosub_engine` (`html` module) | render backend, compositor sink, font system | anything that actually renders |

`RenderConfiguration` extends `ModuleConfiguration`, so a rendering config satisfies both. Parser
test harnesses and fuzz targets implement just `ModuleConfiguration` and stay renderer-free.

How the trait machinery works underneath (the auto-derived `Has*` view traits, why subsystems
bound on those instead of the full config) is covered in [`interface.md`](interface.md).

## The ready-made config

You almost never implement those traits by hand. `DefaultRenderConfig<B, F, S>` is a zero-sized
marker that wires the standard gosub stack (`gosub_html5` + `gosub_css3`) and lets you choose the
parts that vary:

| Param | Meaning | Default |
|---|---|---|
| `B` | render backend | `NullBackend` |
| `F` | font system | `ParleyFontSystem` |
| `S` | compositor sink | `DefaultCompositor` |

With no parameters, `DefaultRenderConfig` is the headless
`DefaultRenderConfig<NullBackend, ParleyFontSystem, DefaultCompositor>` — that's why headless
examples need no backend choice.

## Starting a browser that renders

Pick a backend and font system, alias them once, and hand the alias to the engine. Reuse the
alias everywhere the config type is needed (struct fields, function signatures):

```rust
use std::sync::Arc;
use gosub_engine::{GosubEngine, DefaultRenderConfig};
use gosub_renderer_cairo::{CairoBackend, PangoFontSystem};

// One line per app; this is the only place the backend is named.
type AppConfig = DefaultRenderConfig<CairoBackend, PangoFontSystem>;

let mut engine = GosubEngine::<AppConfig>::new(None, Arc::new(backend), compositor);
```

Every GUI example follows this pattern — see `examples/winit-cairo/main.rs`,
`examples/egui-vello/main.rs`, etc., and [`examples.md`](examples.md) for the full list.

## Available backends

| Backend | Crate | Notes |
|---|---|---|
| `NullBackend` | `gosub_render_pipeline` | headless; no pixels, just geometry + events |
| `CairoBackend` | `gosub_renderer_cairo` | CPU; pairs with `PangoFontSystem` |
| `SkiaBackend` | `gosub_renderer_skia` | CPU or GPU; pairs with `SkiaFontSystem` |
| `VelloBackend` | `gosub_renderer_vello` | GPU via wgpu; generic over a `WgpuContextProvider` |

## Going fully custom

If you need a different CSS/DOM/parser stack (not just a different backend), implement
`ModuleConfiguration` + `RenderConfiguration` on your own zero-sized marker type instead of using
`DefaultRenderConfig`. See the trait definitions in `crates/gosub_engine/src/html.rs`.

> The authoritative API reference lives in the `gosub_engine` crate docs — run
> `cargo doc -p gosub_engine --open` and read the crate-level "Configuration" section.
