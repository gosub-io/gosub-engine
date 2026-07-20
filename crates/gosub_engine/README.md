# gosub_engine

The primary public API of the Gosub browser engine — the crate you depend on to build a
user agent or embed the engine. It ties the parser, CSS, layout, and rendering crates
together behind an async, channel-driven surface: the engine emits `EngineEvent`s, and
the embedder drives it with `EngineCommand` (engine/zone level) and `TabCommand` (per
tab, via `TabHandle`). Work in progress; not yet production-ready.

## The model

- **`GosubEngine`** — create, `start()`/`run()`, `subscribe_events()`, `create_zone()`.
- **Zones** — separate profiles. Each `Zone` owns its cookie jar and storage isolation;
  tabs live inside a zone.
- **Tabs** — browsing contexts driven through `TabHandle`; per-tab runtime state
  (DOM, render pipeline caches) lives in a `BrowsingContext`.
- **`EngineConfig`** — set-once engine configuration via `EngineConfig::builder()`;
  the dynamic settings store (`default_settings()`, backed by `gosub_config`) handles
  runtime-changeable values.
- **`DefaultRenderConfig<Backend, FontSystem, Sink>`** — the zero-sized marker type that
  wires the standard `gosub_html5` + `gosub_css3` stack to your chosen render backend and
  font system. The default is headless (`NullBackend` + `ParleyFontSystem`).

## What lives here

| Area | Contents |
|------|----------|
| `engine/` | `GosubEngine`, zones, tabs, events, `BrowsingContext`, UA policy, settings store |
| `net` | The engine side of networking: dedicated Tokio I/O thread, routing and content decisions, streaming bodies. The fetcher core is the external [gosub-sonar](https://github.com/gosub-io/gosub-sonar) crate |
| `cookies` | `CookieJar` with in-memory, JSON, and SQLite stores (feature `sqlite_cookie_store`, on by default) |
| `storage` | localStorage/sessionStorage services with partitioning policies |
| `resource_pipeline` | Per-asset-kind fetch/parse pipelines (html, css, js, image, font) |
| `html` | `DefaultRenderConfig`, `RenderConfiguration`, document parsing entry points |

Other features: `metrics` (engine metrics module), `ui_eframe` / `winit` / `wayland` /
`x11` (GUI-toolkit integration glue).

## Getting started

Start from [`examples/hello-world.rs`](../../examples/hello-world.rs), then the
[tutorial](../../docs/tutorial.md). The headless path is documented in
[docs/headless.md](../../docs/headless.md) with `gosub-screenshot` as the reference.

## Further reading

- [docs/tutorial.md](../../docs/tutorial.md) — engine / zone / tab concepts, first integration
- [docs/zones-and-tabs.md](../../docs/zones-and-tabs.md) — the runtime model
- [docs/configuration.md](../../docs/configuration.md) — choosing backend and font system
- [docs/cookies.md](../../docs/cookies.md), [docs/datastores.md](../../docs/datastores.md) — isolation and persistence
- [docs/network/](../../docs/network/) — the networking architecture
- [docs/resource-pipeline.md](../../docs/resource-pipeline.md) — async resource loading
