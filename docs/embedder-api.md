# Embedder API

The surface a user agent programs against: what you can send, what you can receive, what works today, and the ordering rules the type system does not enforce.

See [tutorial.md](tutorial.md) for a walkthrough that gets one page on screen, and [zones-and-tabs.md](zones-and-tabs.md) for why the runtime is shaped this way.

``` text
             commands (mpsc, down)                      events (broadcast, up)
   UA ─────────────────────────────► Engine ─► Zone ─► Tab ─────────────────────────► UA
    ▲                                                   │
    │                     frames (ExternalHandle)       │
    └────────────── CompositorSink ◄────────────────────┘
```

Commands travel down over per-object mpsc channels. Events travel up onto a single broadcast bus, tagged with `tab_id`/`zone_id`. Frames do not use the event bus: they go to a `CompositorSink` that you own and read from directly (see [Frames](#frames)).

------------------------------------------------------------------------

## The `unstable-api` feature

Everything reachable in a default build is wired up. Every `EngineEvent` variant you can name is emitted somewhere; every `TabCommand` variant you can name reaches a handler.

Variants that are declared but never emitted, or accepted and dropped with a log line, sit behind the non-default `unstable-api` feature on `gosub_engine`. Matching one without the feature is a compile error rather than a runtime no-op.

``` toml
# For work on the engine itself, or to code against where it is going. Those
# events still never arrive and those commands are still dropped.
gosub_engine = { version = "0.1", features = ["unstable-api"] }
```

Gated entries are marked **· gated ·** in the tables below.

Because the variant set depends on features, `EngineEvent`, `NavigationEvent` and `ResourceEvent` are `#[non_exhaustive]`: every `match` on them needs a `_` arm.

------------------------------------------------------------------------

## Lifecycle and ordering

Several of these steps only work in one order, and the signatures do not say so.

1.  `GosubEngine::new(config, backend, compositor)`. See [configuration.md](configuration.md) for choosing `C`.

2.  `settings()` and `internal_pages()` work before or after `start()`. Internal pages resolve when navigated to, so register them whenever. `net.*` settings are read when a zone makes its first request: set `net.user_agent` before that zone fetches and it applies; a zone already fetching keeps the values it started with.

3.  Call `subscribe_events()` **before** `start()`. It is a `tokio::sync::broadcast` channel, so a receiver only sees messages sent after it subscribed, and `EngineStarted` is emitted synchronously inside `start()`. The same applies to `TabCreated` and `create_tab`.

4.  `start()` returns a future for you to drive; it does not spawn one. You `spawn`, `await`, or `select!` it, which keeps the engine from imposing a threading model on a GTK or winit main loop. A second call returns `EngineError::AlreadyRunning`.

5.  `engine.zone_builder() … .create()`. Everything is optional: with nothing set you get an ephemeral in-memory profile under the engine's default `ZoneConfig` and a fresh id. Set `.config(..)` for limits, `.id(..)` to restore a persisted profile, and `.services(..)` (or the per-field setters: `.storage`, `.cookie_jar`, `.cookie_store`, `.partition_policy`, `.places`) for the isolation boundary.

6.  `zone.tab_builder() … .create().await?` returns a `TabHandle`. Optional: `.url(..)` to navigate at once, `.title(..)`, `.viewport(..)`; and per-tab isolation with `.cookie_jar(..)`, `.storage(..)`, `.partition_key(..)`, `.accept_language(..)`.

7.  Send `ResumeDrawing { fps }`. A fresh tab has `drawing_enabled: false`, so without it no frame is rasterized and no `Redraw` fires however much you navigate. This keeps a backgrounded tab free, and it is the usual reason a first integration renders a blank window.

8.  `engine.shutdown()` drains in-flight requests and flushes cookie and storage state.

------------------------------------------------------------------------

## Commands in: `TabCommand`

Sent with `tab.send(cmd).await`. Some have wrappers on `TabHandle` (`navigate`, `set_title`, `set_viewport`, `set_scroll`, `go_back`, `go_forward`, `render_download`). All are fire-and-forget: the return value says the worker accepted the message, not what it did with it.

| Command | Notes |
|---|---|
| **Navigation and lifecycle** | |
| `Navigate { url }` | A fragment-only change of the current document does not refetch. |
| `LoadHtml { html, base_url }` | Bypasses the network; `base_url` resolves relative subresources. |
| `Reload { ignore_cache }` | Always refetches, fragment or not. |
| `CancelNavigation` | |
| `GoBack` / `GoForward { entry }` / `GoToHistoryEntry { entry }` | History is a tree rather than a stack. `GoForward { None }` takes the most recently visited child. Entry ids come from `NavigationEvent::HistoryChanged`. |
| `RenderDownload { offer }` | Load a pending `DownloadRequested` offer as the page instead of saving it - the override for a misclassified response. Wrapped by `tab.render_download(offer)`. |
| `StartDownload { id, url, target_path, offer }` | `id` is minted by you. With `offer` set, the already-captured body is moved into place and the URL is never re-requested; an offer no longer pending fails rather than fetching. With `offer: None` (save-link-as), `url` is fetched. |
| `CloseTab` | |
| **Rendering control** | |
| `ResumeDrawing { fps }` / `SuspendDrawing` | Nothing paints before the first `ResumeDrawing`. |
| `SetViewport { x, y, width, height }` | CSS px. |
| `SetScroll { x, y }` | Absolute; cancels any scroll animation. See [Scrolling](#scrolling). |
| `SetTitle { title }` | A UA-set title, e.g. for a `LoadHtml` page with no `<title>`. |
| **Input** | |
| `MouseMove` / `MouseDown` / `MouseUp { x, y, button }` | CSS px, viewport-relative. `MouseDown` drives click-to-focus and link activation. |
| `MouseScroll { delta_x, delta_y }` | A delta; the engine decides where it lands and may animate. |
| `KeyDown` / `KeyUp { key, code, modifiers }` | `key` is a `String`; see [Rough edges](#rough-edges). |
| `QueryHitTest { x, y, token }` | `token` is minted by you and echoed back in `HitTestResult`. The input for a native context menu. |
| **Not yet implemented** | |
| `TextInput` · gated · | Committed text for the focused control: IME output, or clipboard contents on paste. The only text-input path; there is no per-character command. Editing lands with M1. |
| `SetCookie` / `ClearCookies` · gated · | Cookies are reachable through the zone's jar today. |
| `SetStorageItem` / `RemoveStorageItem` / `ClearStorage` · gated · | |
| `ExecuteScript` · gated · | Awaiting JS integration, see [javascript.md](javascript.md). |
| `PlayMedia` / `PauseMedia` · gated · | |
| `DumpDomTree` · gated · | |

------------------------------------------------------------------------

## Events out: `EngineEvent`

`subscribe_events()` gives each listener its own receiver on the **control bus**: lifecycle, navigation, downloads, input feedback, crashes. Per-resource detail is not on it - see [Two streams](#two-streams).

| Event | Notes |
|---|---|
| **Engine and zone lifecycle** | |
| `EngineStarted` | Emitted inside `start()`; subscribe first. |
| `ZoneCreated` / `ZoneClosed { zone_id }` | |
| **Rendering** | |
| `Redraw { tab_id }` | Carries no frame data. See [Frames](#frames). |
| **Navigation and resources** | |
| `Navigation { tab_id, event }` | Wraps `NavigationEvent`, below. |
| **Tab state** | |
| `HoverUrl { url }` | `None` when the pointer leaves a link. |
| `CursorChanged { cursor }` | Change-only; resets on navigation. Map to your native cursor. |
| `FocusChanged { focused, editable }` | `editable` is the cue for an on-screen keyboard or IME. |
| `HitTestResult { token, hit }` | Answers `QueryHitTest` with the same token. |
| `FavIconChanged { favicon }` | Raw bytes as served (ICO/PNG/SVG); decode them yourself. Emitted once per committed navigation, and not at all when there is no reachable icon, so keep your placeholder. |
| **Downloads** | |
| `DownloadRequested { offer, url, suggested_filename, ... }` | The navigation was cancelled and the page stayed. Answer with `StartDownload` or `RenderDownload` carrying `offer`, or ignore it. The body is already spooled to a temp file (up to `net.download.max_spool_bytes`). Ignoring the event does not release it: the tab keeps an offer until it is accepted or rendered, until newer offers evict it (8 per tab, oldest first), or until the tab closes. Missed it to `Lagged`? `tab.pending_downloads()` lists what is still there. |
| `DownloadProgress` / `DownloadFinished` / `DownloadFailed { id, ... }` | Correlated by the `DownloadId` you minted. `DownloadProgress` only appears for downloads the engine had to fetch (save-link-as) - an accepted offer is already on disk and finishes at once. `DownloadFailed` may leave a partial file. |
| **Tab lifecycle** | |
| `TabCreated` / `TabClosed { tab_id, zone_id }` | |
| `TabCrashed { tab_id, zone_id, error }` | The worker panicked. The tab is dead: its handle's commands now fail and no further events arrive for it. Show a crash page and offer reload by recreating the tab. |
| **Storage** | |
| `StorageChanged { key, value, scope, origin, ... }` | `value: None` means the key was removed. An empty `key` means the whole area was cleared, not that a key named `""` changed: the underlying `StorageEvent.key: Option<String>` is flattened with `unwrap_or_default()` on the way out. |
| **Not yet implemented** | |
| `TitleChanged` · gated · | Emission arrives with the pending mac-app patches. Until then read `tab.title()`. |
| `LocationChanged` · gated · | Likewise; until then use `NavigationEvent::Finished` or read `tab.url()`. |
| `FrameComplete` · gated · | |
| `TabResized` · gated · | You told the engine the size; it has nothing to add yet. |
| `Warning` / `EngineShutdown` / `BackendChanged` · gated · | |
| `ConnectionEstablished` / `NetworkError` · gated · | Network failures surface as `NavigationEvent::Failed` and `ResourceEvent::Failed`. |
| `CookieAdded` · gated · | |
| `MediaStarted` / `MediaPaused` / `ScriptResult` / `JavaScriptError` · gated · | Awaiting media and JS integration. |

### `NavigationEvent`

Events for the main document. Every event in one navigation carries the same `NavigationId`.

| Variant | Notes |
|---|---|
| `Started { url }` | |
| `Finished { url }` | The final URL after redirects. |
| `Failed { url, error }` | The load failed; `error` is a typed [`LoadError`](#errors). |
| `FailedUrl { url, error }` | The URL string did not parse, as opposed to `Failed`. `error` is `LoadError::InvalidUrl`. |
| `Cancelled { url, reason }` | `CancelReason` distinguishes new navigation, tab close, timeout, and explicit cancel. |
| `HistoryChanged { history }` | The whole `HistorySnapshot`, emitted after the corresponding `Finished`, so shells can drive back/forward menus without querying. |
| `Committed` · gated · | There is no separate commit point yet; a load goes `Started` → `Finished`. |
| `Progress { received_bytes, expected_length, elapsed, .. }` | Load progress of the main document, throttled. |

### `ResourceEvent`

Events for everything else the page loads, delivered as `ResourceUpdate { tab_id, event }` on the separate stream from `subscribe_resource_events()`. Each carries a `RequestId` and a `RequestReference` saying what the resource belongs to.

`Started`, `Redirected`, `Headers`, `Progress`, `Finished`, `Failed` and `Cancelled` are live. `Queued` · gated ·: requests currently go straight to `Started`.

------------------------------------------------------------------------

## Two streams

Events arrive on two independent broadcast channels. They differ in volume, and in what it costs to miss one.

| | `subscribe_events()` | `subscribe_resource_events()` |
|---|---|---|
| Carries | `EngineEvent` - lifecycle, navigation, downloads, input feedback, crashes | `ResourceUpdate { tab_id, event }` |
| Volume | A handful per navigation | Network rate: one `Progress` per chunk, per subresource |
| Buffer | 512 | 4096 |
| Missing one | Can matter - `TabCrashed` and `DownloadRequested` are on here; `tab.pending_downloads()` recovers the latter | `Progress` loss is harmless; a missed `Finished`/`Failed`/`Cancelled` leaves that resource's state unknown, so treat a lag as "re-derive from the next event" |

On a shared bus, a page pulling a hundred subresources can push a `TabCrashed` out of the buffer before a busy shell reads it. Keeping them apart also means a shell that shows no per-resource detail never subscribes to the second channel.

Both are bounded, so both can still lag - see [Contracts](#contracts). Nothing is ordered *between* the two streams; a `ResourceUpdate` and a `NavigationEvent` from the same load have no defined relative order.

------------------------------------------------------------------------

## Errors

Failures arrive as a typed [`LoadError`](https://docs.rs/gosub_engine), not an opaque error string. The discriminant is what decides which error page a shell shows, and whether retrying could help.

| Variant | Means | Retry? |
|---|---|---|
| `Blocked { reason }` | Refused before or instead of loading. `reason` is a `BlockReason`: `Policy`, `MixedContent`, `UrlPolicy`, `UnsupportedScheme`. | No |
| `InvalidUrl { message }` | The URL string did not parse. | No |
| `Network { message }` | The transfer failed - DNS, connection, TLS, HTTP. | Maybe |
| `Timeout { message }` | The request did not finish within the time limit. | Maybe |
| `Io { message }` | A local I/O failure: writing a download, opening storage. | Maybe |
| `Cancelled { message }` | A new navigation, the tab closing, or an explicit cancel. | n/a |
| `Content { message }` | The bytes arrived but could not be made into a document. | No |
| `Other { message }` | Unclassified. | Unknown |

It implements `Display`, so code that only prints the error needs no change from the days when this was an `Arc<anyhow::Error>`. It is `#[non_exhaustive]`: match with a `_` arm, because variants will be added as the engine learns to tell failures apart.

------------------------------------------------------------------------

## Frames

Two things arrive per frame, and only one of them is the frame.

When a tab finishes rendering, the worker calls `CompositorSink::submit_frame(tab_id, handle)` with the backend's `ExternalHandle` (CPU pixels, a tile cache, or a GPU texture id, depending on the backend), then emits `EngineEvent::Redraw { tab_id }`.

The sink is the data channel. You constructed it, passed it to `GosubEngine::new`, and read the current frame out of it when you paint. The event is only the wakeup: it carries a `TabId` and nothing else, and maps onto your toolkit's invalidate call.

``` rust
// GTK4: the event invalidates, the draw handler reads the sink.
EngineEvent::Redraw { .. } => drawing_area.queue_draw(),
```

Present from the sink, not from the event. The bus drops messages for slow consumers, and a dropped wakeup costs one coalesced repaint because the next one still finds the newest frame in the sink. A dropped frame would cost the frame.

------------------------------------------------------------------------

## Scrolling

Both sides track the offset. The engine is authoritative: it clamps to the real page height, restores the saved offset when you traverse to a history entry, and runs the smooth-scroll animation. A shell will usually also keep its own offset so it can shift already-rasterized tiles at input rate without a round trip through the tab worker.

The two converge through the frame. The tile cache handed to the sink carries the offset it was composited at, so a shell drawing at its own local offset is corrected as frames arrive.

Which command you send says who is deciding:

| | `MouseScroll { delta_x, delta_y }` | `SetScroll { x, y }` |
|---|---|---|
| Carries | A delta | An absolute offset |
| Engine may animate | Yes | No; cancels any animation in flight |
| Use for | Raw wheel and trackpad input | Scrollbar drag, UA-side kinetic scrolling, restored session |

Do not mix the two within one gesture. An absolute set landing mid-animation will fight the animator.

------------------------------------------------------------------------

## Reading tab state

Read it off the handle when you need an answer now, such as painting a tab strip, saving a session, or enabling a Back button:

``` rust
tab.url()             // Option<Url>, None before the first commit
tab.title()           // String, empty until the document supplies one
tab.can_go_back()     // bool
tab.can_go_forward()  // bool
```

Every tab-scoped event - `Navigation`, `Redraw`, the input feedback and download events, `TabCreated`/`TabClosed`/`TabCrashed` - carries a `TabId`; `zone.tab(id)` turns one back into a `TabHandle`, so a shell does not need its own id-to-handle map. `EngineStarted` and the zone events do not.

These are synchronous and never block on the worker. They read a snapshot the worker republishes as it commits navigations, so during an in-flight navigation they still describe the previous document, which is what an address bar should show.

Subscribe to events instead when you need the moment something changes, or detail the accessors do not carry: the full history tree (`HistoryChanged`), per-resource progress, download lifecycle, crashes.

------------------------------------------------------------------------

## Contracts

-   **Downloads arrive as an offer, not a question, and the bytes are already yours.** When a response is not a renderable page (content-type or `Content-Disposition` says download, or the type is unknown) the engine decides on its own: it cancels the navigation, leaves the page in place, and emits `EngineEvent::DownloadRequested`. Answer with `StartDownload` or ignore it. There is no blocking ask; the offer is the ask, and `RenderDownload` is the answer that overrides the engine's classification. The response body is spooled to a temp file at that point, so accepting places what was already fetched instead of issuing a second request. An ignored offer is kept until newer offers evict it or the tab closes. Offers ride the bounded control bus: after a `Lagged`, call `tab.pending_downloads()` - anything listed can still be accepted or rendered by its `offer` id.
-   **`ResumeDrawing` before anything paints.** See [Lifecycle](#lifecycle-and-ordering) step 7.
-   **Tokens are yours to mint.** `HitTestToken` and `DownloadId` are chosen by the embedder and echoed back. Uniqueness is your responsibility; the engine does not check.
-   **`TabCrashed` means the tab is gone.** Stop sending to that handle and recreate the tab.
-   **Handle `Lagged`.** Both streams are bounded, so `broadcast::Receiver::recv` returns `Err(Lagged(n))` when your consumer fell behind and `n` messages were dropped. Continue the loop; treating it as `Closed` and breaking kills your event handling on the first busy page. Lagging the control bus is worth logging - it means something like a `TabCrashed` may have gone missing.
-   **Zone services are the isolation boundary.** Two zones sharing a `StorageService` or cookie jar are not isolated, whatever their `ZoneId`s say.

------------------------------------------------------------------------

## Rough edges

-   **`Redraw` is still on the control bus.** It fires per frame, so a shell scrolling at 60fps puts real traffic alongside `TabCrashed`. It sits there because shells overwhelmingly want it in the same loop as navigation, and because a dropped one costs a coalesced repaint. If it becomes a problem it wants a third stream, not a bigger buffer.
-   **The input model is thin.** `KeyDown.key` is a `String` rather than a typed key, and there is no IME composition, touch, or pointer id. `TextInput` is declared but not yet handled, so text editing does not work at all. `FocusChanged { editable }` is the hook for an on-screen keyboard, waiting on the other half.
-   **`LoadError::Network` still lumps DNS, connect and TLS together**, because the HTTP client reports them as one error type. Telling them apart means inspecting `reqwest::Error` inside the network layer. Separately, the *resource* stream is coarser than the navigation one: `ResourceEvent::Failed` can only ever be `Network`, because `NetEvent::Failed` carries a bare `anyhow::Error` where the navigation path gets a typed `NetError`. Both want a change in gosub-sonar; `LoadError` is `#[non_exhaustive]` so neither will be breaking.
-   **`#![deny(missing_docs)]` is commented out** at the top of `lib.rs`. Some of what is public is public by accident.

------------------------------------------------------------------------

## See also

-   [tutorial.md](tutorial.md) — the same material as a walkthrough, with a runnable `examples/tutorial.rs`.
-   [zones-and-tabs.md](zones-and-tabs.md) — why zones and tab workers are shaped this way.
-   [configuration.md](configuration.md) — choosing the render backend, font system, and the rest of `C`.
-   [examples.md](examples.md) — the GTK4, winit and egui shells.
-   [headless.md](headless.md) — driving the same API with no window.
