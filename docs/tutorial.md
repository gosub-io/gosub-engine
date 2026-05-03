# Getting started with the Gosub engine

This tutorial walks you through the core lifecycle of the Gosub engine. By the
end you will have a working program that starts the engine, opens a tab,
navigates to a URL, and reacts to events - the same pattern used by every
embedding that builds on Gosub.

The companion runnable example lives at [`examples/tutorial.rs`](../examples/tutorial.rs).
Run it directly with:

```bash
cargo run --example tutorial -- https://example.com
```

---

## Key concepts

Before touching any code, it helps to understand the five things you interact
with constantly.

### Engine

`GosubEngine` is the central hub. It owns the event bus, networking stack, and
render backend. You create it once, call `start()`, and then drive it entirely
through commands and events. The engine itself runs on Tokio, so your
application needs an async runtime.

### Zone

A **Zone** is an isolated browsing profile. It owns its own cookies, local
storage, session storage, and tabs. Think of it like a browser profile or a
private window. Multiple zones can coexist in one engine instance - you might
use this for separate user accounts, or to sandbox untrusted content.

### Tab

A **Tab** is a single browsing context, like a browser tab. Every tab lives
inside exactly one zone and inherits that zone's cookies and storage unless you
override them. You control a tab through its `TabHandle` by sending
`TabCommand` values.

### Events

The engine is **event-driven**. It communicates with your application by emitting
`EngineEvent` values over a channel. Your application receives these events and
reacts - rendering a frame, updating a progress bar, following a redirect. You
never poll the engine directly; you wait for events.

### DecisionRequired

When the engine fetches a URL it needs to know what to do with the response: render
the page, or save the file? It can't decide on its own (it doesn't know your
UI), so it pauses the navigation and emits a `NavigationEvent::DecisionRequired`
event. Your application must reply with `TabCommand::SubmitDecision` carrying
either `Action::Render` or `Action::Download`. If you do not reply, the navigation
stalls indefinitely.

---

## Step-by-step walkthrough

### 1. Add the dependency

In your `Cargo.toml`:

```toml
[dependencies]
gosub_engine = { git = "https://github.com/gosub-io/gosub-engine", package = "gosub_engine" }
tokio = { version = "1", features = ["full"] }
```

### 2. Create the engine

```rust
use std::sync::{Arc, RwLock};
use gosub_engine::{EngineConfig, GosubEngine};
use gosub_engine::render::{backends::null::NullBackend, DefaultCompositor};

let backend = NullBackend::new().expect("null backend");
let mut engine = GosubEngine::new(
    Some(EngineConfig::default()),
    Arc::new(backend),
    Arc::new(RwLock::new(DefaultCompositor::default())),
);

let join_handle = engine.start().expect("cannot start engine");
```

`NullBackend` skips all pixel rendering - useful for headless scenarios or
whenever you just want navigation and events without a visible window. Swap it
for `CairoBackend` or `VelloBackend` to get an actual rendered surface.

Subscribe to events **before** creating any zones or tabs, so you don't miss
events emitted during setup:

```rust
let mut events = engine.subscribe_events();
```

### 3. Create a zone

```rust
use gosub_engine::cookies::DefaultCookieJar;
use gosub_engine::storage::{
    InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService,
};
use gosub_engine::zone::{ZoneConfig, ZoneServices};

let services = ZoneServices {
    storage: Arc::new(StorageService::new(
        Arc::new(InMemoryLocalStore::new()),
        Arc::new(InMemorySessionStore::new()),
    )),
    cookie_store: None,
    cookie_jar: Some(DefaultCookieJar::new().into()),
    partition_policy: PartitionPolicy::None,
};

let mut zone = engine.create_zone(ZoneConfig::default(), services, None)?;
```

`InMemoryLocalStore` and `InMemorySessionStore` give you ephemeral storage that
disappears when the zone is dropped. For persistent cookies, pass a
`CookieStore` in `ZoneServices::cookie_store` and set `cookie_jar` to `None`.

### 4. Open a tab

```rust
use gosub_engine::render::Viewport;
use gosub_engine::tab::TabDefaults;

let tab = zone.create_tab(
    TabDefaults {
        viewport: Some(Viewport::new(0, 0, 1280, 800)),
        ..Default::default()
    },
    None,
).await?;
```

`create_tab` returns a `TabHandle`. Hold on to it - you need it to send
commands and to match events back to the right tab.

### 5. Navigate

```rust
use gosub_engine::events::TabCommand;

tab.send(TabCommand::Navigate {
    url: "https://example.com".into(),
}).await?;
```

This queues a navigation request. The engine starts fetching asynchronously and
begins emitting `EngineEvent::Navigation` events.

### 6. Event loop

```rust
use gosub_engine::events::{EngineEvent, NavigationEvent};

loop {
    tokio::select! {
        Ok(ev) = events.recv() => {
            match ev {
                EngineEvent::Navigation { event, .. } => match event {
                    NavigationEvent::Started { url, .. } =>
                        println!("started:  {url}"),
                    NavigationEvent::Finished { url, .. } => {
                        println!("finished: {url}");
                        break;
                    }
                    NavigationEvent::Failed { url, error, .. } => {
                        println!("failed:   {url}  ({error})");
                        break;
                    }
                    NavigationEvent::DecisionRequired {
                        nav_id, decision_token, ..
                    } => {
                        // Always render (never download) in this example.
                        tab.cmd_tx.send(TabCommand::SubmitDecision {
                            nav_id,
                            decision_token,
                            action: gosub_engine::Action::Render,
                        }).await?;
                    }
                    _ => {}
                },
                EngineEvent::Redraw { .. } => {
                    // Composite a frame into your window here.
                }
                _ => {}
            }
        }
        _ = tokio::signal::ctrl_c() => break,
    }
}
```

The `DecisionRequired` arm is important: without it the navigation stalls
because the engine is waiting for your reply.

### 7. Shutdown

```rust
engine.shutdown().await?;
if let Some(handle) = join_handle {
    let _ = handle.await;
}
```

Always shut down cleanly. This drains in-flight network requests and flushes
any pending state before the process exits.

---

## Full example

The steps above are assembled into a single, runnable file:

```
examples/tutorial.rs
```

```bash
# Navigate to a URL and print events until loading is complete
cargo run --example tutorial -- https://news.ycombinator.com
```

---

## What to try next

| Goal | Where to look |
|---|---|
| Handle multiple tabs | [`examples/multi-tab.rs`](../examples/multi-tab.rs) |
| Render with GTK4 / Cairo | [`examples/gtk-cairo/`](../examples/gtk-cairo/) |
| Render with wgpu / Vello | [`examples/egui-vello/`](../examples/egui-vello/) |
| Parse HTML directly (no engine) | [`examples/html5-parser.rs`](../examples/html5-parser.rs) |
| Understand all the crates | [`docs/crates.md`](crates.md) |
| Use the component tools | [`docs/binaries.md`](binaries.md) |
