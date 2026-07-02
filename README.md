# Gosub Browser Engine

An embeddable, async browser engine written in Rust.

Join us on our development [Zulip chat](https://chat.developer.gosub.io), or our
[Discord server](https://chat.gosub.io) for general chat. If you'd like to contribute, start with
the [contribution guide](CONTRIBUTING.md).


## About

Gosub is a modular, embeddable browser engine. The primary entry point is `GosubEngine` in the
[`gosub_engine`](crates/gosub_engine) crate. You provide a render backend and a compositor; the
engine owns a multi-zone/tab model, an async networking stack, cookie and storage isolation per
zone, and an event bus. Your user-agent (UA) drives everything via `TabCommand` and reacts to
`EngineEvent`.

**Core components:**

| Crate | Role |
|---|---|
| `gosub_engine` | `GosubEngine` — the unified entry point |
| `gosub_interface` | Shared traits wiring the components together (the config system) |
| `gosub_html5` | HTML5 tokenizer / parser |
| `gosub_css3` | CSS3 tokenizer / parser |
| `gosub_net` | Networking stack (async, streaming, per-zone) |
| `gosub_taffy` | Flexbox / grid layout (Taffy) |
| `gosub_lattice` | CSS table layout |
| `gosub_render_pipeline` | Render pipeline — stages, tiling, compositor |
| `gosub_renderer_cairo` | Cairo render backend (CPU) |
| `gosub_renderer_skia` | Skia render backend (CPU / GPU) |
| `gosub_renderer_vello` | Vello / wgpu render backend (GPU) |
| `gosub_fontmanager` | Font system — text shaping and measurement |
| `gosub_jsapi` | Browser Web API implementations (console, fetch, DOM, …) |
| `gosub_v8` | V8 JavaScript engine bindings |
| `gosub_config` | Configuration store |

For the full crate listing see [`docs/crates.md`](docs/crates.md).


## Status

The engine is under active development. What works today:

- **Multi-zone / multi-tab model** — zones isolate cookies and storage; tabs are controlled via `TabCommand`
- **Async networking** — streaming HTTP fetcher with priority queues, inflight coalescing, redirect handling, and per-zone cookie isolation
- **Event-driven UA interface** — `EngineEvent` (navigation, resource, redraw) flows out; `TabCommand` / `EngineCommand` flow in
- **HTML5 and CSS3 parsing** — spec-compliant parsers for both
- **Pluggable render backends** — Null (headless), Cairo (GTK4), Skia, Vello (wgpu)


## Documentation

Start here, then dig into the topic you need.

**Getting started**

- [Tutorial](docs/tutorial.md) — start the engine, open a tab, navigate, handle events
- [Configuration](docs/configuration.md) — choosing a render backend and font system
- [Running the examples](docs/examples.md) — headless, GUI (winit / GTK4 / egui), and component tools
- [WebAssembly](docs/webassembly.md) — compile and run the engine in the browser
- [Development](docs/development.md) — tests and benchmarks

**Reference**

- [Crates](docs/crates.md) — the workspace crate layout
- [Component tools](docs/binaries.md) — the standalone `cargo run --bin …` tools

**Architecture**

- [Networking — architecture](docs/net-architecture.md) and [design notes](docs/net-design.md)
- [Cookies](docs/cookies.md)
- [Storage (local / session)](docs/datastores.md)
- [Pump](docs/pump.md) — moving HTTP stream data to targets
- [Render pipeline](docs/render-pipeline/README.md)


## Contributing

We welcome contributions. Because the engine is still taking shape, a lot of work is exploratory
— building proofs-of-concept, reading specs, and making architectural decisions — rather than
pure coding.

Join us on [Zulip](https://chat.developer.gosub.io) or [Discord](https://chat.gosub.io) before
diving in; it will save you time and help us keep things coordinated. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the details.
