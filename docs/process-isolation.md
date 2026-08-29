# Process isolation

How the engine splits itself into sandboxed processes, what an embedder must do to
turn that on, and how to observe and test it. This page is the overview; the font
side of the story is in [fonts.md](fonts.md) ("Confinement tiers"), and the render
pipeline the isolated renderers run is described under
[render-pipeline/](render-pipeline/README.md).

Everything here is **on by default on Linux** for an embedder that follows the
contract below, and off elsewhere until those platforms are verified (see
[Settings](#settings)). The renderer process family is **Linux only**; the
network and decoder processes also run on macOS and Windows with
platform-appropriate confinement.

## Why

Page content is untrusted input fed to large parsers: HTML, CSS, images, fonts.
The engine's answer is the same as every modern browser's: run that parsing and
rendering in processes that hold no secrets and have had almost every privilege
removed, so a bug in a codec or layout is a crashed throwaway process, not a
compromised browser. Three properties fall out of the design and are worth
naming, because the code defends them everywhere:

- **Renderers hold no capabilities.** No network, no filesystem, no devices, no
  new processes. Everything a render needs either arrives over its one socket or
  was inherited copy-on-write before lockdown.
- **The broker never trusts the wire.** Lengths, dimensions, counts and file
  descriptors coming from a child are claims to validate, not facts.
- **Page content never renders in the broker.** Once renderer isolation is on, a
  failed out-of-process render leaves the tab blank and tells the embedder
  (`EngineEvent::RendererCrashed`) rather than quietly running the page in the
  trusted process. Only the engine's own `gosub:`/`about:` pages are exempt.

## The process model

```
broker (the embedder's process: engine, zones, tabs, navigation, compositing)
├── gosub-net           network stack; the engine's only network capability
├── gosub-vault         the cookie jars (`security.cookie_vault`); talks to gosub-net directly
├── gosub-storage       localStorage areas on files (embedders using `ServiceLocalStore`)
├── gosub-decoder       raster image decoding; one throwaway process per image
└── gosub-forksrv       the fork server: fonts warmed once, renderers forked from it
    ├── pidns-anchor    PID 1 of the renderers' private PID namespace
    ├── renderer-<id>   resident renderer for one (zone, site)
    └── renderer-<id>   … one per site with open tabs
```

Every child renames itself for `ps`/`pstree` (comm + cmdline), so the tree above
is what you actually see on a running system.

Every child starts with an allowlisted environment (`HOME`, `TMPDIR`, locale,
`XDG_*`, `SSL_CERT_*`, `RUST_LOG`, `GOSUB_*` and little else), no stdin, and
only the descriptors the spawner named — everything else is marked
close-on-exec first. Each has a task ceiling (`pids.max`) and a memory ceiling
sized for its role in its own cgroup, where cgroup v2 is delegated. Roles that
may write files (the storage service) or read and reach the network (gosub-net)
refuse to start on a kernel without Landlock rather than run unscoped; the
engine then falls back in-process and says so.

- **gosub-net** is the only process allowed to reach the network. Tabs never
  fetch: their loads are brokered requests that the I/O runtime performs with the
  tab's cookies attached on the way through — tab code never sees a cookie value.
  It keeps the host network namespace and nothing else (fresh user, IPC and UTS
  namespaces), and `socket()` there is limited to `AF_INET`/`AF_INET6`: unix,
  netlink and the rest fail with `EAFNOSUPPORT`, so D-Bus, agents and the like
  are out of reach.
- **gosub-vault** holds the cookie jars, in the least-authority profile of the
  model (no network, no files, no devices). The rule behind it: no one process
  should hold both large secrets and a large hostile-input surface, and the
  broker deserializes frames from every other child. Tabs and the embedder API
  see an ordinary jar that forwards; the network process gets its own line to
  the vault, so the cookie values attached to requests and the `Set-Cookie`
  headers coming back flow between those two and never through the broker.
  That line is not trusted to name zones: before dispatching a request the
  broker grants the vault a random per-request ticket bound to the tab's zone
  and document, the network process asks under the ticket, and the vault
  answers from the grant. The broker also accepts a jar snapshot only for a
  zone with a live request or a mutation of its own outstanding.
  Persistence is brokered: the vault sends a snapshot of a zone's jar after
  every change and the broker writes it through the zone's cookie store, so
  the vault never opens a file and any embedder-supplied store works. An
  embedder-supplied *jar* stays where the embedder put it.
- **gosub-storage** serves `localStorage` from one JSON file per
  `(zone, partition, origin)` area, under the service profile (baseline plus
  `openat`, Landlock-scoped to its directory). A zone whose `StorageService`
  holds a `FileLocalStore` is routed through it at zone creation
  (`security.storage_service`, one process per directory); other store kinds
  stay in-process. Area names are stamped by the broker,
  page keys stay inside the file, values and areas are capped. Session
  storage stays in the broker.
- **gosub-decoder** is spawned per image, fed bytes, and exits with pixels (or
  an intrinsic size, when only that was asked). SVG is rasterized there at its
  intrinsic size, without fonts — a parsed tree never leaves the process — so
  the tightest profile applies. Whatever it answers is bounded and checked on
  receipt; a decode it refuses or cannot start is a failed image, never one
  decoded in the caller instead.
- **gosub-forksrv** exists for font warm-up, not fork speed: it builds and
  prepares the configured font system once (see fonts.md), confines itself, and
  forks renderers that inherit the warmed state copy-on-write. It also lazily
  unshares a PID namespace; whatever forks first becomes that namespace's PID 1
  and must outlive every renderer, which is the anchor's whole job. A forked
  renderer closes the fork server's broker link and the anchor pipe before its
  own lockdown — fork ignores `FD_CLOEXEC`, and either fd in a renderer would
  let it forge broker traffic or kill every sibling.
- **renderer-\<id\>** processes are *resident*: one per `(zone, site)` — site
  being scheme + eTLD+1, Chromium's definition — hosting every tab of that site
  in that zone, alive until the site's last tab closes. They are forked from the
  fork server on demand; the broker then talks to each renderer directly over a
  socket handed across with `SCM_RIGHTS`.

Confinement is layered per role: seccomp allowlists (default-deny, violations
die with a SIGSYS naming the syscall and, for path-taking calls, the path),
Landlock filesystem scoping where a role needs any files at all, namespace
unsharing (network, IPC, UTS, and PID for the renderer family), rlimits
(committed memory, fd count, no core dumps, lowered priority), and non-dumpable
processes. Renderers get the strictest profile their font system permits — the
**confinement tier** — which is the font system's own answer; fonts.md explains
the tiers and why they exist.

## The embedder contract

1. **`gosub_engine::child_process::dispatch_with::<AppConfig>()` must be the
   first statement of `main()`.** The engine spawns children by re-executing the
   embedder's own binary with a role flag; dispatch is what routes those
   invocations into the child role instead of the embedder's startup. Plain
   `dispatch()` works for the net and decoder roles but cannot run the fork
   server, which needs the concrete `RenderConfiguration` type. The call also
   registers itself: an engine whose process never dispatched turns the three
   `security.*` process settings off at `start()` with one warning naming the
   omission, so a child can never re-exec into the embedder's startup (a child
   that was somehow started that way refuses to spawn further processes too).
2. **Flip the settings before `start()`** (they are read once at startup):
   `security.network_process`, `security.image_decoder_process`,
   `security.renderer_process`.
3. **Provide a forked rasterizer.** Isolated renderers rasterize on the CPU in
   the child; `RenderConfiguration::forked_tile_rasterizer` must return one.
   `DefaultRenderConfig` does so when the engine's `cairo-tiles` (or
   `skia-tiles`) feature is on, whatever the broker's own backend; a custom
   configuration returns e.g. `CairoRasterizer::with_font_system`. With no
   rasterizer the renderer tier is not started at all (warning, in-process
   rendering) rather than producing geometry without pixels.
   Tip from the test embedder (`examples/mini-browser`): use the *null* backend
   for the embedder's own rendering, so that if the isolated path ever broke and
   fell back, tabs would go blank rather than quietly rendering unisolated.
4. **Listen for `EngineEvent::RendererCrashed { zone_id, site, tabs, error }`.**
   A dead renderer is replaced transparently on its tabs' next render — most
   pages recover on their own — but the embedder may want to show something
   meanwhile, and when the error names the fork server the tab could not be
   rendered at all.

## How a page gets rendered

The tab worker (broker) fetches the document and keeps only its source: it
does not run the HTML parser on page content. It sends the renderer
`Navigate { tab, html, url, viewport, scroll_y, known_tiles, … }`; the
renderer parses, styles and lays out the page — fetching stylesheets, web fonts
and images through the broker, see below — reports the document's title and
icon URL in its render summary (the broker raises `TitleChanged` and fetches
the icon from that), then keeps that laid-out page
retained per tab and rasterizes **only the raster window** around the viewport
(scroll ± one viewport height, the same policy as the in-process tile budget).

- **Scroll**: `Scroll { tab, y }` re-uses the retained page — no parse, no
  layout — rasterizes what came into the window, and announces tiles it let go
  of (`Evict`) once they drift more than three viewports away. The broker merges
  the result into its tile set.
- **Hover**: `Hover { tab, node }` restyles just the old and new hover chains
  and repaints only the tiles the hovered element covers.
- Both run **asynchronously**: the broker starts the exchange on a helper
  thread and keeps compositing the tiles it already holds; the result is merged
  on a later frame. One pass in flight per tab; a navigation invalidates stale
  results by generation.
- Renders on one renderer are strictly serial (one socket, request/reply), so
  same-site tabs take turns — see [Known limits](#known-limits-and-roadmap).

**Pixels travel as sealed shared memory.** The renderer rasterizes a tile,
copies it once into a `memfd`, seals it (`F_SEAL_WRITE|SHRINK|GROW`), sends the
header plus the fd over the socket, and drops its copy; the broker validates
size against seals and maps the pages, compositing zero-copy from then on.
Sealing closes the time-of-check/time-of-use hole; the one-fd-at-a-time
discipline on both sides means a page of any height streams through a 128-fd
limit. Tiles are deduplicated by content hash: a tile the broker already holds
is neither rasterized nor shipped again.

**Bodies stream across the network boundary.** A request that asks for its
body as it arrives gets it that way from the network process too: the response
head travels in-band, a sealed shared-memory ring (`gosub_ipc::ring`, 256 KiB
window) follows as a file descriptor, and the network process writes the body
into the ring as it reads it from the socket while the broker drains it into
the same `SharedBody` an in-process fetch would produce. Neither side ever
holds the whole body for the transport; a consumer that stops draining stalls
the producer (backpressure) and, after a bounded wait, ends the stream. Linux
only; elsewhere the network process buffers as before.

**Subresources are brokered.** A confined renderer cannot fetch, so it sends
`NeedResource { url, deferred }` and blocks; the broker performs the load where
identity and cookies live and replies with bytes. Stylesheets and fonts are
blocking (layout cannot proceed without them). Images ask **deferred**: the
broker answers immediately — bytes if cached, `Pending` otherwise — fetches in
the background, and re-renders the tab when the bytes land, so a render never
waits on an image download.

**Memory is bounded at three levels**: decodes are refused above hard limits
and huge images are kept downscaled (with their true intrinsic size preserved
for layout); the renderer's decoded-image cache holds at most ~96 MiB, evicting
LRU pixels and re-decoding on use from kept encoded bytes; and a renderer
retains at most 3 laid-out pages (LRU tab's page is dropped; its next scroll
comes back empty, which makes the broker re-render it). The renderer family
runs under a 1 GiB `RLIMIT_DATA`; other children get 512 MiB.

**Crashes** are detected eagerly (a non-blocking liveness probe on every idle
renderer, ~4×/s) and by any failing exchange; the pool replaces the process,
emits `RendererCrashed`, and the tab re-renders in the replacement. A wedged
renderer is bounded by the exchange timeouts (60 s for renders — generous on
purpose, so a slow page is never mistaken for a wedged process — 10 s for control
traffic).

## What a page may load

Two policies sit in the broker's I/O runtime, on every subresource a page
loads, in both the in-process and the network-process arrangement. Both are
decided from the tab's *own* document (the top-level URL the broker recorded
at navigation), never from anything the requester sent.

- **Private-network protection** (`net::ssrf`). A subresource of a document on
  the public internet may not reach loopback, private, link-local, CGNAT,
  multicast or the other reserved ranges - the classic SSRF through
  `<img src="http://169.254.169.254/…">`. Navigations are never restricted, and
  a document that itself lives on the private network may load its neighbours.
  The decision and the connection are one step: such requests go through a
  *strict* fetcher (one per zone, and one in the network process) whose DNS
  resolver refuses a name if *any* answer is private and which classifies IP
  literals - including the `2130706433` / `0x7f000001` / `127.1` spellings and
  the NAT64/6to4/IPv4-mapped IPv6 embeddings - at every redirect hop. There is
  no second lookup for a rebinding attack to poison.
- **Opaque-response blocking** (`net::orb`). A cross-origin response body only
  reaches a renderer when it is something a page may embed: images, media,
  CSS, scripts, fonts. HTML, JSON and XML - by declared type, or by what the
  first bytes sniff as - stay in the broker; the requester sees an error. This
  is what makes per-site renderer processes mean something: another origin's
  data never enters a renderer's address space through an `<img>` or `<script>`
  tag. Mislabelled images (a PNG served as `text/plain`) are recognised by
  their bytes. There is no CORS input yet; every load the engine issues today
  is a no-cors subresource load.

## Settings

| Setting | Default | Effect |
|---|---|---|
| `security.network_process` | on (Linux) | Network stack in its own sandboxed process. Falls back in-process with a warning (network code is trusted engine code; the sandbox is defense in depth). |
| `security.image_decoder_process` | on (Linux) | Raster decoding in a throwaway process per image. Falls back in-process with a warning. |
| `security.storage_service` | on (Linux) | A zone's `localStorage` served by the storage process when its local store is a `FileLocalStore` (one process per directory). Other stores stay in-process. |
| `security.cookie_vault` | on (Linux) | The cookie jars in their own sandboxed process, with a direct line from the network process (see the process model). Linux only; falls back to in-process jars with a warning. |
| `security.renderer_process` | on (Linux, `Full`-tier font systems) | The fork server + resident renderer machinery described above. **No fallback for page content**: if it cannot start, pages simply render in-process from the beginning (with a warning at startup); once it *has* started, a page that cannot be rendered out of process stays blank. Linux only. |

The defaults are *offers*: at `start()` the engine keeps each one only where
it can apply, and says what it decided at `info` level (or `warn`, when the
embedder set the value explicitly and it still cannot apply):

- none of them without `child_process::dispatch()` in this process;
- the network and decoder processes on Linux only, until the macOS/Windows
  backends have run in CI (set them explicitly to try them there);
- the renderer tier only for font systems that answer `Full` (Parley,
  cosmic-text) and configurations with a forked rasterizer. `FontPathsReadable`
  font systems (Skia, Pango) get the exec-per-render tier - a fresh process for
  every render, no resident renderers - and must opt in explicitly.

What defaults-on buys is crash and memory isolation of untrusted parsing and
rendering. Data isolation (opaque-response blocking, SSRF policy, the cookie
vault) is tracked separately; see [Known limits](#known-limits-and-roadmap).

## Observing it

- `ps`/`pstree` show the named processes. Renderers are `renderer-<id>`, on
  purpose without the site or URL: the process list is readable by every
  user on the machine. Which renderer serves which site is on `/renderers`
  and in the telemetry. `NSpid` in `/proc/<pid>/status` shows a renderer's
  pid inside the private PID namespace.
- With the engine's `metrics` feature, `127.0.0.1:9090` serves `/metrics`
  (timing aggregates), `/renderers` (the pool: site, pid, tabs, RSS), and
  `/events` — the **telemetry firehose**, newline-delimited JSON of engine
  events: `remote.navigate`/`remote.scroll`/`remote.hover` (exchange time,
  tiles, per-stage renderer timings), `net.load` (every brokered fetch:
  outcome, status, bytes, duration), `remote.resource` (every subresource a
  renderer asked for), `tab.frame`, `tab.invalidate` (why a full render
  happened), `renderer.memory`. `tools/telemetry-viewer/index.html` is a
  standalone page that visualizes the stream.

## Testing it

- `cargo test -p gosub_engine --test process_isolation --features cairo-tiles`
  — the end-to-end suite (net, decoder, fork server, resident renderer
  lifecycle/scroll-window/hover/crash/soak, engine wiring), driven through the
  `isolation-harness` binary, which dispatches child roles like a real embedder.
- `cargo test -p gosub_sandbox` — sandbox unit tests plus enforcement probes
  that verify each profile actually blocks what it claims to.
- Harness tools (not tests): `render-file` replays a saved page through a
  forked renderer (`render-file-locked` runs it in-process under the lockdown,
  so `gdb` catches sandbox violations with a full backtrace); `engine-soak`
  loads real sites through the whole engine and reports per-site costs;
  `engine-stress` runs many tabs with continuous random input and a live log
  (`GOSUB_STRESS_TABS`, `GOSUB_STRESS_PACE_MS`, `GOSUB_STRESS_SEED`);
  `renderer-soak` hammers one renderer with hundreds of navigations and checks
  memory stays flat.
- `examples/mini-browser` is a minimal winit embedder with everything switched
  on; `Ctrl+P` prints the live process tree and the renderer pool.

## Known limits and roadmap

- **Same-site tabs serialize** on their shared renderer: a slow render delays
  the site's other tabs (measured, deliberate for now; the fix is request
  interleaving, planned together with the input/JS protocol evolution).
- **Tiles are CPU pixels.** GPU texture ids cannot cross processes and a
  sandboxed renderer must never touch the GPU; the plan is broker-side texture
  upload first, then out-of-process raster (the renderer ships paint command
  lists; stage 6 runs where the GPU lives).
- **`FontPathsReadable` font systems** (fontconfig-based: Pango, Skia) get a
  weaker arrangement: no fork server, a throwaway renderer exec'd per render,
  with read-only Landlock-scoped font paths. See fonts.md.
- The vault's `document.cookie` view (`visible_only`) has no consumer yet; it
  starts to matter when scripts can read cookies. A zone using an
  embedder-supplied jar is not vaulted.
