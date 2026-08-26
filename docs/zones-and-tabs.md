# Zones and tabs

How the engine organizes running state: one `GosubEngine`, containing isolated **zones**, each containing **tabs** that run as independent tasks. This is the architecture view; for a hands-on walkthrough see [tutorial.md](tutorial.md), and for the API surface itself see [embedder-api.md](embedder-api.md).

``` text
GosubEngine<C>                        (owns backend, compositor, font system, event bus)
 ├── Zone "Home"                      (profile: own cookies, storage, identity)
 │     ├── Tab ──► TabWorker task     (own DOM, navigation, render state)
 │     └── Tab ──► TabWorker task
 └── Zone "Work"                      (a second, fully isolated profile)
       └── Tab ──► TabWorker task
```

## Why zones

A zone is a **browser profile/container** (in the Firefox-containers sense): it encapsulates everything that constitutes an identity on the web --- cookies, local/session storage, and runtime services --- so "Work" and "Home" zones browsing the same site are two different users as far as that site can tell. Zones also carry UI metadata (title, icon, color, description) so a user agent can render them as visible containers.

Isolation is the default; sharing is opt-in per data class via `SharedFlags` (autocomplete, bookmarks, passwords, cookie jar).

A zone is created with:

-   **`ZoneConfig`** --- limits and settings (e.g. maximum tabs);
-   **`ZoneServices`** --- the isolation boundary made concrete: a `StorageService` (local + session stores --- see [datastores.md](datastores.md)), an optional cookie store/jar (see [cookies.md](cookies.md)), and a `PartitionPolicy` describing how storage is keyed (e.g. per-origin partitioning);
-   an optional fixed `ZoneId` (a UUID --- pass one to restore a persisted zone across runs).

Internally the zone builds a `ZoneContext` that flows down to every tab it creates: the services above plus the engine-wide pieces --- event channel, network I/O channel, the render backend, compositor sink, and the single shared font system (all concrete types per the config `C`, see [configuration.md](configuration.md)).

## The engine around them

`GosubEngine<C: RenderConfiguration>` owns what zones share:

-   the **render backend** and **compositor sink** (one instance each, per the config);
-   the **font system** --- one instance, handed to both layout and rasterization so measurement and drawing agree (see [fonts.md](fonts.md));
-   the **network I/O thread** --- networking runs on its own thread with an `IoChannel`; responses are routed back to the right tab via a request-reference map;
-   the **command channel** (`EngineCommand`, mpsc into the engine run loop --- crate-internal; embedders use methods on `GosubEngine`) and the **event bus** (`EngineEvent`, a tokio broadcast channel --- `subscribe_events()` gives every listener its own receiver), and a **separate resource stream** (`ResourceUpdate`, via `subscribe_resource_events()`) so per-chunk fetch progress cannot crowd must-not-miss events like `TabCrashed` out of the bounded control bus.

The command/event flow is strictly layered: commands travel *down* (UA → engine → zone → tab, each over its own mpsc channel), events travel *up* onto the one broadcast bus, tagged with their `tab_id`/`zone_id` so the UA can demultiplex.

## Tabs

A tab is a browsing context. `zone.create_tab(TabDefaults, overrides)` spawns a **`TabWorker`** --- a dedicated tokio task owning everything about that page --- and returns a `TabHandle`:

``` rust
pub struct TabHandle {
    pub tab_id: TabId,
    pub(crate) cmd_tx: TabChannel,   // mpsc of TabCommand into the worker
    pub(crate) sink: Arc<TabSink>,   // tab outputs, read via url()/title()/...
}
```

The handle is the *entire* control surface: everything is a `TabCommand` message (convenience methods like `navigate()` / `set_viewport()` wrap `send()`). The command set falls into four groups:

| Group | Commands |
|-------|----------|
| Navigation | `Navigate`, `Reload`, `CancelNavigation`, `SubmitDecision` |
| Lifecycle | `CloseTab`, `SetTitle` |
| Drawing | `ResumeDrawing { fps }`, `SuspendDrawing`, `SetViewport`, `SetScroll` |
| Input | `MouseMove/Down/Up/Scroll`, `KeyDown/Up` |

Inside the worker:

-   **Navigation is a cancellable async job.** Each navigation gets a `NavigationId` and a `CancellationToken`; the fetch/parse runs concurrently and reports back over a oneshot channel, so a new `Navigate` (or `CancelNavigation`) cleanly aborts the old one. Progress is published as `NavigationEvent`s (`Started`, `Finished`, `Failed`, ...).
-   **Downloads**: when a response arrives that isn't obviously a renderable page (content-type/disposition says download, unknown type, ...), the worker classifies it itself via `decide_handling`, cancels the navigation so the current document stays, and emits `EngineEvent::DownloadRequested` as an offer the UA can accept with `StartDownload` or ignore. The body is spooled to a temp file at that point, so accepting places bytes already fetched rather than re-requesting the URL --- which would be wrong for a POST-produced or single-use download --- and an offer that is never accepted releases the file. (`NavigationEvent::DecisionRequired` --- the engine *asking* instead of deciding --- is declared but never emitted, and sits behind the `unstable-api` feature; only its answering half is wired.)
-   **Drawing is pull-based and rate-limited.** Nothing paints until the UA sends `ResumeDrawing { fps }`; the worker then runs a tick loop at that rate, driving the [render pipeline](render-pipeline/README.md) (per the backend's `RasterStrategy`) and submitting finished frames to the compositor sink. `SuspendDrawing` stops the ticks --- a backgrounded tab costs nothing.
-   **The sink carries frames; `Redraw` is only the wakeup.** Every `submit_frame` to the compositor sink is followed by an `EngineEvent::Redraw { tab_id }` that carries no frame data. UAs paint by reading the current frame out of the sink they own, and map `Redraw` onto their toolkit's invalidate call. The event bus is a `broadcast` channel that drops messages for slow consumers, so a lost wakeup costs one coalesced repaint while a lost frame would cost the frame.
-   **Scroll position is tracked on both sides.** The engine is authoritative: it clamps to the page, restores per-history-entry offsets, and runs the smooth-scroll animation. A UA may keep its own offset as well, so it can shift already-rasterized tiles at input rate without a round trip through the worker; the tile cache handed to the sink carries the offset it was composited at, which is how the two converge. Send `MouseScroll` (a delta; the engine decides where it lands and may animate) for raw wheel and trackpad input, and `SetScroll` (an absolute offset, cancelling any animation) for a scrollbar drag, UA-side kinetic scrolling, or a restored session. Don't mix the two within one gesture.
-   The worker owns the tab's `BrowsingContext` --- document, styles, pipeline caches, scroll state --- none of which is reachable from outside except through commands and events. What a shell needs synchronously is mirrored onto the tab's `TabSink` and read back through `TabHandle::{url, title, can_go_back, can_go_forward}`, so building a tab strip or restoring a session does not mean replaying the event stream.

## Why this shape

-   **Isolation by construction.** Zone state lives in `ZoneServices`; a tab can only reach what its `ZoneContext` hands it. Cross-zone leaks would require explicit plumbing.
-   **One slow tab can't stall the rest.** Each `TabWorker` is its own task; heavy layout or a hung fetch affects only that tab. Networking is off on its own thread entirely.
-   **Message passing over shared state.** The UA ↔ engine boundary is channels in both directions, so a GUI event loop (winit, GTK, egui) and the engine's tokio runtime never lock each other --- the [headless tool](headless.md) drives the exact same interface without a window.
-   **Compile-time config all the way down.** `ZoneContext` carries the *concrete* backend/compositor/font types of `C: RenderConfiguration` --- no dynamic dispatch on the hot rendering path and no possibility of a zone mixing components from different configs.
