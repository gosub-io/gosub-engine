# Module configuration

The Gosub engine is **generic over a configuration type** that names every swappable component as an associated type. You pick the implementations you want --- CSS system, DOM, HTML parser, render backend, compositor --- once, at compile time, and the engine is monomorphized against that choice. The engine code itself only ever talks to the component *traits*, so it never knows (or needs to know) which concrete implementations you chose.

``` rust
use gosub_engine::{GosubEngine, DefaultConfig};
use gosub_renderer_cairo::CairoBackend;

// "The default parse stack, rendering with Cairo."
let engine = GosubEngine::<DefaultConfig<CairoBackend>>::new(
    None,                                  // engine settings (see note below)
    Arc::new(CairoBackend::new()),         // the backend instance
    Arc::new(RwLock::new(DefaultCompositor::default())),
);
```

This document describes it from the perspective of someone *embedding* the engine.

------------------------------------------------------------------------

## The mental model

A configuration is a **zero-sized marker type** that implements one or two traits. The traits have no methods --- they only carry associated types that say "use *this* implementation for *that* role":

| Role | Associated type | Example implementation |
|------|-----------------|------------------------|
| CSS parsing & cascade | `CssSystem` | `gosub_css3::system::Css3System` |
| DOM storage | `Document` | `gosub_html5::…::DocumentImpl<Self>` |
| HTML5 parsing | `HtmlParser` | `gosub_html5::parser::Html5Parser<'static, Self>` |
| Render backend | `RenderBackend` | `CairoBackend` / `SkiaBackend` / `VelloBackend` / `NullBackend` |
| Frame sink | `CompositorSink` | `DefaultCompositor` |

Because the engine is generic over your config, swapping any of these is a one-line change in one place, checked by the compiler --- there is no runtime registry or plugin lookup.

> **You must compile in what you name.** Writing `type RenderBackend = CairoBackend` only works if your crate depends on `gosub_renderer_cairo`. The config *is* the wiring, and the wiring has to exist at build time.

------------------------------------------------------------------------

## The two traits

The roles are split across **two** traits, by layer:

### `ModuleConfiguration` --- the parse/document stack

`gosub_interface::config::ModuleConfiguration`

``` rust
pub trait ModuleConfiguration: Clone + Debug + PartialEq + Send + Sync + 'static {
    type CssSystem: CssSystem;
    type Document: Document<Self>;
    type HtmlParser: Html5Parser<Self>;
}
```

This is everything needed to parse HTML+CSS into a DOM. It does **not** mention rendering, so parse-only tools (test harnesses, fuzz targets, the standalone parser binaries) can implement just this without ever depending on a renderer crate.

The narrow `Has*` view traits (`HasCssSystem`, `HasDocument`, `HasHtmlParser`) are **derived automatically** from a `ModuleConfiguration` --- you never implement them by hand.

### `EngineConfig` --- the full engine stack

`gosub_engine::html::EngineConfig`

``` rust
pub trait EngineConfig: ModuleConfiguration<Document = DocumentImpl<Self>> {
    type RenderBackend: RenderBackend + Send + Sync;
    type CompositorSink: CompositorSink;
}
```

`GosubEngine<C>` requires `C: EngineConfig`. It extends `ModuleConfiguration` with the runtime render components, and pins `Document = DocumentImpl<Self>` (the HTML parser produces that concrete document type, so the document and parser are a coupled pair rather than independently swappable).

Render components live here, not on `ModuleConfiguration`, specifically so that parse-only configs stay free of any renderer dependency.

------------------------------------------------------------------------

## `DefaultConfig<B, S>` --- the easy path

Most embedders want the standard gosub parse stack (gosub_css3 + gosub_html5) and only care about *which backend* to render with. For that, use the provided generic config:

``` rust
pub struct DefaultConfig<B = NullBackend, S = DefaultCompositor>;
```

-   `B` --- the render backend.
-   `S` --- the compositor sink (almost always `DefaultCompositor`).

It implements both `ModuleConfiguration` (Css3System + DocumentImpl + Html5Parser) and `EngineConfig` (`RenderBackend = B`, `CompositorSink = S`) for you. So:

-   `DefaultConfig` --- headless: `DefaultConfig<NullBackend, DefaultCompositor>`.
-   `DefaultConfig<CairoBackend>` --- render with Cairo, default compositor.
-   `DefaultConfig<SkiaBackend>` --- render with Skia.
-   `DefaultConfig<VelloBackend<MyWgpuProvider>>` --- render with Vello.

------------------------------------------------------------------------

## Recipes

### 1. Headless (no rendering)

``` rust
use gosub_engine::{GosubEngine, DefaultConfig};
use gosub_render_pipeline::render::{backends::null::NullBackend, DefaultCompositor};

let engine = GosubEngine::<DefaultConfig>::new(
    None,
    Arc::new(NullBackend::new()),
    Arc::new(RwLock::new(DefaultCompositor::default())),
);
```

`GosubEngine` defaults its type parameter to `DefaultConfig`, so in type position you can also just write `GosubEngine` (e.g. a struct field `engine: GosubEngine`).

### 2. Pick a render backend (the common case)

``` rust
use gosub_engine::{GosubEngine, DefaultConfig};
use gosub_renderer_skia::SkiaBackend;

let engine = GosubEngine::<DefaultConfig<SkiaBackend>>::new(
    None,
    Arc::new(SkiaBackend::new()),
    Arc::new(RwLock::new(DefaultCompositor::default())),
);
```

If you store the engine in your own struct, name the full type there too:

``` rust
struct App {
    engine: GosubEngine<DefaultConfig<SkiaBackend>>,
    // …
}
```

### 3. A fully custom config (swap CSS/DOM too)

When you want to replace the CSS system (or other parse-stack pieces) as well, define your own marker type and implement both traits:

``` rust
#[derive(Clone, Debug, PartialEq)]
struct MyConfig;

impl ModuleConfiguration for MyConfig {
    type CssSystem  = MyCustomCssSystem;        // your CSS implementation
    type Document   = DocumentImpl<Self>;       // keep the gosub DOM
    type HtmlParser = Html5Parser<'static, Self>;
}

impl EngineConfig for MyConfig {
    type RenderBackend  = CairoBackend;
    type CompositorSink = DefaultCompositor;
}

let engine = GosubEngine::<MyConfig>::new(None, Arc::new(CairoBackend::new()), compositor);
```

`MyCustomCssSystem` only has to implement the `gosub_interface::css3::CssSystem` trait --- the engine never refers to it by name, only through that trait.

------------------------------------------------------------------------

## What is and isn't swappable yet

| Component        | Status |
|------------------|--------|
| `CssSystem`      | ✅ config-driven |
| `Document`       | ✅ config-driven; coupled to the parser, in practice `DocumentImpl` |
| `HtmlParser`     | ✅ config-driven |
| `RenderBackend`  | ✅ config-driven |
| `CompositorSink` | ✅ config-driven |
| `FontSystem`     | ⏳ not yet a config member; currently a concrete font system |
| `NetworkStack`   | ⏳ not yet a config member |

Runtime backend switching (flipping Cairo↔Skia↔Vello without recompiling) is still available separately via the `gosub_renderer_dynamic` crate --- set `type RenderBackend = DynamicRenderBackend` and switch inside that one type.

------------------------------------------------------------------------

## Why the components are passed to `new()`

`GosubEngine::<C>::new(settings, backend, compositor)` still *takes* the backend and compositor **instances**, even though their types come from the config. This is deliberate: some backends need runtime context to construct (e.g. Vello needs a wgpu device/queue). The config fixes the *type* (`Arc<C::RenderBackend>`), and the compiler verifies the instance you pass matches it --- you can't accidentally hand a `SkiaBackend` to a config that declared `CairoBackend`.

------------------------------------------------------------------------

## Config vs. settings

Two different things, two different names --- don't confuse them:

-   **Config** = *which components* (the traits): `ModuleConfiguration` and `EngineConfig` (`gosub_engine::html::EngineConfig`). This is what this document is about.
-   **Settings** = *tuning knobs*: `EngineSettings` (`gosub_engine::EngineSettings`) --- channel capacities, limits, etc. It's the first argument to `GosubEngine::new` (`None` uses the defaults), built via `EngineSettings::builder()`.

Rule of thumb: **Config picks the pieces; Settings tunes them.**
