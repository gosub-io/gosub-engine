# gosub_interface

The trait contracts of the Gosub browser engine. This crate is the dependency-inversion heart
of the workspace: it defines *what* every major engine component does, while the component
crates (`gosub_html5`, `gosub_css3`, the renderer backends, ...) define *how*. Component crates
depend on this crate and implement its traits — never on each other — and generic engine code
names components only through a configuration type.

## The configuration model

A client picks its component set at compile time by defining one zero-sized marker type that
implements `ModuleConfiguration`, naming each component as an associated type:

```rust
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;                    // gosub_css3
    type Document = DocumentImpl<Self>;             // gosub_html5
    type HtmlParser = Html5Parser<'static, Self>;   // gosub_html5
}
```

There is no runtime registry: the engine is generic over `C: ModuleConfiguration`, so naming a
component implies a Cargo dependency on the crate that provides it. Subsystem code doesn't
bound on the full configuration but on narrow `Has*` view traits (`HasDocument`,
`HasCssSystem`, ...), which are derived automatically through blanket impls.

## What lives here

| Module | Contract |
|--------|----------|
| `config` | `ModuleConfiguration` and the `Has*` view traits |
| `document`, `node` | `Document<C>`: a storage-agnostic DOM, addressed by `NodeId` handles |
| `html5` | `Html5Parser<C>`: `ByteStream` → document, with recoverable parse errors |
| `css3` | `CssSystem`: parsing, selector matching + cascade, value access |
| `layout` | `Layouter<C>` / `LayoutTree<C>` and per-node geometry |
| `font`, `font_system` | `FontSystem`: register + measure fonts (never draw) |
| `render` | The pipeline ↔ backend contract: `RenderBackend`, `CompositorSink`, `RenderList`, tiles, `PixelFormat`, viewports |
| `input` | The small `InputEvent` vocabulary UAs feed into the engine |

## What deliberately does *not* live here

- **Implementations.** Everything in this crate is a trait or a plain value type.
- **Render configuration.** `ModuleConfiguration` is parse-only by design; the rendering
  extension (`RenderConfiguration`) lives in `gosub_engine`, so parser test harnesses and fuzz
  targets never pull in renderer crates.
- **Types from higher crates.** This crate sits at the bottom of the dependency graph. Where a
  contract must carry a type it cannot name, it uses a documented `Any` seam and the owning
  crate downcasts on the other side.

## Further reading

- [docs/interface.md](../../docs/interface.md) — the full architecture view: every trait
  family, the configuration model, and the type-erasure seams
- [docs/configuration.md](../../docs/configuration.md) — the embedder's view: how to pick
  components when using the engine
