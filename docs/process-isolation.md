# Process isolation

How the engine splits itself into sandboxed processes, what an embedder must do to
turn that on, and how to observe and test it. This page is the overview; the font
side of the story is in [fonts.md](fonts.md) ("Confinement tiers"), and the render
pipeline the isolated renderers run is described under
[render-pipeline/](render-pipeline/README.md).

Everything here is **on by default on Linux** for an embedder that follows the
contract below - except renderer processes for a font system that reads font
files while shaping (Pango, Skia: `FontPathsReadable`), which an embedder opts
into - and off elsewhere until those platforms are verified (see
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
`XDG_*`, `SSL_CERT_*`, `RUST_LOG`, `GOSUB_*` except the `GOSUB_DUMP_*` debug
dumps, and little else), no stdin, and only the descriptors the spawner named
— everything else is marked close-on-exec first. Each has a task ceiling
(`pids.max`) and a memory ceiling sized for its role in its own cgroup, where
cgroup v2 is delegated; a forked renderer is moved out of the fork server's
cgroup into one of its own as soon as the fork server announces it, so one
site's renderer never trips a cap shared with another's. Roles that may write
files (the storage service), read and reach the network (gosub-net), or read
font files while rendering page content (the font-readable renderer tier)
refuse to start on a kernel without Landlock rather than run unscoped; the
engine then falls back in-process and says so.

- **gosub-net** is the only process allowed to reach the network. Tabs never
  fetch: their loads are brokered requests that the I/O runtime performs with the
  tab's cookies attached on the way through — tab code never sees a cookie value.
  It keeps the host network namespace and nothing else (fresh user, IPC and UTS
  namespaces), and `socket()` there is limited to `AF_INET`/`AF_INET6`: unix,
  netlink and the rest fail with `EAFNOSUPPORT`, so D-Bus, agents and the like
  are out of reach. Its files are scoped to the read-only resolver and CA paths
  before its async runtime exists: Landlock binds the thread that applies it and
  the threads created after it, and the runtime's workers are where every
  response is parsed. Its escape audit runs on one of those workers for the
  same reason.
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
  embedder-supplied *jar* stays where the embedder put it. A vault that dies
  is respawned on the next cookie operation: every open zone is reopened from
  its store's last snapshot (a zone without a store loses its session
  cookies) and the network process is handed a fresh line over its own link.
- **gosub-storage** serves `localStorage` from one JSON file per
  `(zone, partition, origin)` area, under the service profile (baseline plus
  `openat`, Landlock-scoped to its directory). A zone whose `StorageService`
  holds a `FileLocalStore` is routed through it at zone creation
  (`security.storage_service`, one process per directory); other store kinds
  stay in-process. Area names are stamped by the broker,
  page keys stay inside the file, values and areas are capped. Session
  storage stays in the broker. A service that dies is respawned on the next
  request, which is retried once; its state was on disk all along.
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

What each process can still reach is not left to reading: `isolation-harness
escape-audit` asks every child of a running engine - net, decoder, vault,
storage, the fork server, a renderer forked for it and every resident renderer
- to run the escape audit inside itself after its real spawn and lockdown:
open `/etc/passwd`, `$HOME`, `/proc`, create in `/tmp`, sockets of each
family, fork, exec, unshare, signal the broker, ptrace, executable memory,
stat by path, plus what it inherited: every descriptor beyond its links
(a forked renderer still holding the fork server's broker link, say) and
every environment key the spawner's allowlist would not have passed. Each
attempt's outcome is checked against the role's design - where the kernel
would refuse on its own (pid 1 as a signal target, a user namespace on a host
that blocks them) the row demands the seccomp trap itself - and the suite
fails on any violation (`gosub_sandbox::audit`; the seccomp trap is caught
and turned into an errno so the audit survives its own attempts).

Confinement is layered per role: seccomp allowlists (default-deny, violations
die with a SIGSYS naming the syscall and, for path-taking calls, the path;
the one exception is `stat` by path, which a role without files gets back as
`EPERM` so a library's `exists()` probe on page input is a missing file and
not a dead renderer, while `fstat` on its own descriptors keeps working in
every spelling libc and std use. The filter's default is a trap rather than a
kill so the signal can be reported; a compromised child can install its own
handler and survive the signal, but the refused syscall still never runs),
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
   registers itself: an engine whose process never dispatched turns all five
   `security.*` process settings off at `start()` with one warning naming the
   omission, so a child can never re-exec into the embedder's startup (a child
   that was somehow started that way refuses to spawn further processes too).
2. **Confine the embedder's own process next:**
   `gosub_engine::child_process::lock_down_broker(&[profile_dir, ...])`,
   right after dispatch and before any thread, the logger or the engine
   exist. It limits this process's filesystem writes to the temp dir plus
   the directories named (the profile with its cookie store, localStorage and
   places; downloads; logs) and removes the escalation syscalls. The
   children's sandboxes are the boundary against page content; this is the
   one against a bug in the broker reaching the rest of the account. The
   mini-browser does it; the harness runs every engine scenario under it.
3. **Choose before `start()`** (the settings are read once at startup). One
   switch, `security.process_isolation` (`GosubEngine::set_process_isolation`),
   decides everything: `false` is the single-process engine, as it always
   was; `true` asks for every component process, and says which cannot
   apply here. `set_process_isolation` writes the setting through the
   configured settings storage, so with a persistent adapter it holds for
   later runs until the key is removed: the user's preference. An embedder's
   `--single-process` flag is a choice for this run and maps to
   `set_process_isolation_for_this_run`, which writes nothing back and
   outranks the stored value (`examples/mini-browser` has it, and
   `--isolated`). Left unset, the five
   settings decide on their own: `security.network_process`,
   `security.image_decoder_process`, `security.renderer_process`,
   `security.cookie_vault`, `security.storage_service` - for an embedder that
   wants, say, the service processes without the renderer tier.
4. **Provide a forked rasterizer.** Isolated renderers rasterize on the CPU in
   the child; `RenderConfiguration::forked_tile_rasterizer` must return one.
   `DefaultRenderConfig` does so when the engine's `cairo-tiles` (or
   `skia-tiles`) feature is on, whatever the broker's own backend; a custom
   configuration returns e.g. `CairoRasterizer::with_font_system`. With no
   rasterizer the renderer tier is not started at all (warning, in-process
   rendering) rather than producing geometry without pixels.
   Tip from the test embedder (`examples/mini-browser`): use the *null* backend
   for the embedder's own rendering, so that if the isolated path ever broke and
   fell back, tabs would go blank rather than quietly rendering unisolated.
5. **Listen for `EngineEvent::RendererCrashed { zone_id, site, tabs, error }`.**
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
- **Input**: `Input { tab, scroll_y, known_tiles, event }` applies one user
  action to the retained page where its DOM is - a press, a release, a key,
  committed text, a picker's answer - and answers like a scroll: the tiles it
  changed, then `Rendered` carrying what the input asked of the broker as
  *effects* (see [Input on a remote page](#input-on-a-remote-page)). Focus,
  a caret and typed text are a repaint of the tiles under the control; a
  toggle, a dropdown or a resize lays the page out again, after which the
  pass ships the window by content hash against `known_tiles` - the broker's
  tile memory, sent along as on a navigate - so only tiles whose pixels
  changed travel, and the renderer evicts what the broker held that the new
  layout no longer accounts for.
- **Resize**: `Resize { tab, viewport_width, viewport_height, dpr, scroll_y,
  known_tiles }` lays the retained page out again at the new size - no parse -
  and ships the result by content hash like an input pass that re-laid the
  page. Two things keep it cheap. When the style environment is unchanged (no
  `@media` condition flipped, no sheet reads the viewport) the renderer runs
  taffy over the layout tree it kept instead of restyling and rebuilding it,
  the shortcut the in-process pipeline takes for geometry-only damage; the pass
  reports `resize.geometry` instead of `resize.layout`. And it rasterizes the
  viewport alone rather than the window around it: a drag produces many sizes,
  each replacing every tile, so the broker records the tight band and asks for
  the margin (one scroll pass) only once no further size is waiting. The
  broker sends a resize instead of a navigate when its viewport changes on a
  page it adopted from a resident renderer; only the latest size waits behind
  the pass in flight, so the renderer lays out the sizes it has time for
  rather than every one. Meanwhile the broker composites the tiles it holds,
  at their old geometry, until the pass lands.
- All four run **asynchronously**: the broker starts the exchange on a helper
  thread and keeps compositing the tiles it already holds; the result is merged
  on a later frame. One pass in flight per tab; input that arrives meanwhile
  queues in order (pointer moves keep the last, wheel notches add up, nothing
  else coalesces) and goes out after a pending resize and before a pending
  hover; a navigation invalidates stale results by generation and drops the
  queue.
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

**Requests report back as if fetched in-process.** The network process has no
tabs and no event bus, so its fetcher's observer sends every event the engine
reports - name resolved, connected, request sent, headers, progress, finished,
failed, cancelled - back over the link as `FromNet::Event { tag, … }`, flattened
to plain data with a failure already classified where its typed cause exists.
The broker registers its own observer for the request before the `Fetch` goes
out (the same `EngineEventEmitter` an in-process fetch gets, wrapped for timing
where that is compiled in), replays each event into it, bounded - strings cut,
headers capped, a body preview no longer than the broker asked for - and
guarantees exactly one terminal event: a request the broker cancelled, or whose
process died, is ended here. For a streamed body the events keep arriving after
the `Reply` that carried the head. A renderer's subresource requests name a
document reference of the brokered loader's own; the I/O side records which tab
that is, so they reach the embedder's resource stream attributed like the
page's own. The embedder's request log and anything built on it (Beacon's
developer panel, its activity strip) therefore see the same under isolation as
without.

**Subresources are brokered.** A confined renderer cannot fetch, so it sends
`NeedResource { url, deferred }` and blocks; the broker performs the load where
identity and cookies live - with the page's `Referer` and `Accept-Language`,
as the page's own fetch would, and `file:` only for a page that itself came
from disk - and replies with bytes. The private-network and opaque-response
policies below are decided from the document the request is for, which the
broker stamps on it, so a page still shown keeps asking as itself while the
tab loads the next one. The renderer also gets the user's media preferences
(`prefers-color-scheme`, reduced motion, the DPR media environment) with
every render request, since it has no settings of its own to read.
Stylesheets and fonts are
blocking (layout cannot proceed without them). Images ask **deferred**: the
broker answers immediately — bytes if cached, `Pending` otherwise — fetches in
the background, and re-renders the tab when the bytes land, so a render never
waits on an image download.

**Memory is bounded at three levels**: decodes are refused above hard limits
and huge images are kept downscaled (with their true intrinsic size preserved
for layout); the renderer's decoded-image cache holds at most ~96 MiB, evicting
LRU pixels and re-decoding on use from kept encoded bytes; and a renderer
retains at most 3 laid-out pages (LRU tab's page is dropped; its next scroll
comes back empty, which makes the broker re-render it). The renderer family -
forked and exec'd alike - runs under a 1 GiB `RLIMIT_DATA`; other children get
512 MiB. On the broker's side a tab keeps at most 512 MiB and 20 000 of its
renderer's tiles (the oldest go, and the renderer ships them again if the page
still needs them), and the link text a page's hit regions carry is bounded
per URL (a longer one is dropped whole, never cut) and per page.

### Input on a remote page

The broker has no DOM for a page a resident renderer retains, so what needs
one happens in the renderer and the broker becomes a relay that judges the
answers. What stays in the broker, from the hit regions the renderer ships:
hover styling (`Hover`), the cursor shape, the link under the pointer for the
status bar and the context menu, and scrolling. What goes out as `Input`:
every press and release, committed text, picker answers, and keys - all of
them while something on the page is focused, and all but the page-scrolling
ones (arrows, page keys, Home, End, Space) otherwise, which scroll the page in
the broker as they would with nothing focused. Pointer moves and wheel
notches go out only while the renderer **holds the pointer**: a slider thumb,
a textarea grip or scrollbar, a selection being dragged out, an open
dropdown; it says so with a `Capture` effect, and the broker then skips its
own hover processing and page scrolling until the release. A link click
goes out too (it ends the renderer's focus like any press) and comes back as a
navigation request. The exec-per-render tier retains no page and gets no
input.

What comes back are **requests, never state**: `Focus { focused, editable,
bounds }`, `Cursor`, `Navigate { url, post, body }` (a form submission, or a
link activated by click or keyboard), `Picker { kind, bounds, value, min,
max, step }`, `ClipboardWrite`, `PasteRequested`, `Capture`. Each is tagged
with what produced the pass and judged before the broker acts: a navigation
faces the rule a hit-region link gets (`http`/`https`, `file` only from a
`file` page), a cursor is believed from a pointer press or move only, a
clipboard write only from a pass whose key was Ctrl/Meta+C or +X and a paste
request only from +V, a picker's bounds are clamped to the viewport. On the
way in the client bounds them like hit regions: more than 16 effects in one
pass ends the exchange as a crash, a URL past the hit-text bound or a form
body past 1 MiB drops its navigation whole, a rectangle that is not a number
drops its effect. A pass is held to a 5 s deadline in the renderer, against
the 120 s a render gets. The renderer keeps a page's focus and gestures with the page:
one it let go of under the retention cap answers input with `no_page`, and
the broker renders it afresh, unfocused.

Measured on the workstation (`engine-remote-latency`, Cairo tiles in both
modes): a keystroke reaches the frame in about 4.6 ms in-process and 8 ms out
of process, the difference being the round trip and the renderer's paint of
the one tile under the field; a checkbox toggle takes about 150 ms either
way, which is the layout, not the protocol.

**Crashes** are detected eagerly (a non-blocking liveness probe on every idle
renderer, ~4×/s) and by any failing exchange; the pool replaces the process,
emits `RendererCrashed`, and the tab re-renders in the replacement. A wedged
renderer is bounded by the exchange timeouts (60 s for renders — generous on
purpose, so a slow page is never mistaken for a wedged process — 10 s for control
traffic). A renderer also bounds itself: a one-shot one arms a
120 s deadline before its lockdown and a resident one arms the same per
request, so a page that loops in layout ends the process rather than keeping
a core busy at its memory limit until the engine exits. That bounds a buggy
page, not a hostile renderer, which can disarm its own timer: the broker
holds a pidfd for every resident renderer - opened at spawn and verified
against `/proc` to be the fork server's child, never taken on the fork
server's word - and kills through it whatever it has given up on.

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
  their bytes. `application/octet-stream` is sniffed like an absent type, and
  the types ORB never sniffs (`text/csv`, `text/event-stream`, archives, PDF,
  office documents) are blocked on the label alone. There is no CORS input yet; every load the engine issues today
  is a no-cors subresource load.

## Settings

| Setting | Default | Effect |
|---|---|---|
| `security.process_isolation` | unset | Set, it decides the five below for the run: `false` is single-process, `true` requests all five (explicitly, so each that cannot apply warns). Unset, the five decide on their own. |
| `security.network_process` | on (Linux) | Network stack in its own sandboxed process. Falls back in-process with a warning (network code is trusted engine code; the sandbox is defense in depth). |
| `security.image_decoder_process` | on (Linux) | Raster decoding in a throwaway process per image. Falls back in-process with a warning. |
| `security.storage_service` | on (Linux) | A zone's `localStorage` served by the storage process when its local store is a `FileLocalStore` (one process per directory). Other stores stay in-process. |
| `security.cookie_vault` | on (Linux) | The cookie jars in their own sandboxed process, with a direct line from the network process (see the process model). Linux only; falls back to in-process jars with a warning. |
| `security.renderer_process` | on (Linux, `Full`-tier font systems) | The fork server + resident renderer machinery described above. **No fallback for page content**: if it cannot start, pages simply render in-process from the beginning (with a warning at startup); once it *has* started, a page that cannot be rendered out of process stays blank. Input to such a page goes to the renderer too (see [Input on a remote page](#input-on-a-remote-page)). Linux only. |

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
- With the engine's `metrics` feature and `telemetry.metrics_enabled` on (it
  is off by default), `127.0.0.1:9090` serves `/metrics` (timing
  aggregates), `/renderers` (the pool: site, pid, tabs, RSS), and `/events`
  — the **telemetry firehose**, newline-delimited JSON of engine
  events: `remote.navigate`/`remote.media`/`remote.scroll`/`remote.hover`/`remote.input`/`remote.resize`
  (exchange time, tiles, per-stage renderer timings), `net.load` (every brokered fetch:
  outcome, status, bytes, duration), `remote.resource` (every subresource a
  renderer asked for), `tab.frame`, `tab.invalidate` (why a full render
  happened), `renderer.memory`. The server's own `/` is a page that
  visualizes the stream: open `http://127.0.0.1:9090/` in a browser. The
  server answers only requests whose `Host` is a loopback name, so a page
  elsewhere cannot read the stream by pointing its own name at 127.0.0.1; it
  is still readable by every local user while it is on, which is why it is
  off by default.

## Testing it

- `cargo test -p gosub_engine --test process_isolation --features cairo-tiles`
  — the end-to-end suite (net, decoder, fork server, resident renderer
  lifecycle/scroll-window/hover/crash/soak, input on the renderer directly and
  through the engine - every control on one page, a tall page laid out again,
  the retention cap, a renderer that lies about its effects - and the engine
  wiring), driven through the `isolation-harness` binary, which dispatches
  child roles like a real embedder.
- `cargo test -p gosub_sandbox` — sandbox unit tests plus enforcement probes
  that verify each profile actually blocks what it claims to.
- Harness tools (not tests): `render-file` replays a saved page through a
  forked renderer (`render-file-locked` runs it in-process under the lockdown,
  so `gdb` catches sandbox violations with a full backtrace); `engine-soak`
  loads real sites through the whole engine and reports per-site costs;
  `engine-stress` runs many tabs with continuous random input and a live log
  (`GOSUB_STRESS_TABS`, `GOSUB_STRESS_PACE_MS`, `GOSUB_STRESS_SEED`);
  `renderer-soak` hammers one renderer with hundreds of navigations and checks
  memory stays flat; `engine-remote-latency <backend> [local|remote]` times
  keystrokes and toggles from the tab command to the next composited frame
  in either mode.
- `examples/mini-browser` is a minimal winit embedder with everything switched
  on; `Ctrl+P` prints the live process tree and the renderer pool.

## Known limits and roadmap

- **Same-site tabs serialize** on their shared renderer: a slow render delays
  the site's other tabs, and a keystroke in one waits behind another's render
  (measured, deliberate for now; the fix is request interleaving in the
  renderer, planned together with the script protocol).
- **A re-layout after input costs a full layout** (about 150 ms on a page of
  forty paragraphs, in-process and out alike); incremental layout in the
  pipeline is the follow-up, not the protocol.
- **Input has no window-blur command yet**, so a renderer's focus and gestures
  outlive the window's; and the bounds a `Focus` effect carries are not yet
  used to scroll the control into view.
- **Tiles are CPU pixels.** GPU texture ids cannot cross processes and a
  sandboxed renderer must never touch the GPU; the plan is broker-side texture
  upload first, then out-of-process raster (the renderer ships paint command
  lists; stage 6 runs where the GPU lives).
- **`FontPathsReadable` font systems** (fontconfig-based: Pango, Skia) get a
  weaker arrangement: no fork server, a throwaway renderer exec'd per render,
  with read-only Landlock-scoped font paths (scoped before the font system is
  built, since Landlock binds threads and the font stack may start one; a
  kernel without Landlock gets no such renderer at all). See fonts.md.
- A `file:` page may embed any readable local file, as in Chromium and
  Firefox; with a renderer exploit that is a local file read. Keep
  `net.file.enabled` off where local pages are not needed. See
  [security-assessment.md](security-assessment.md) for this and the other
  accepted limits.
- The vault's `document.cookie` view (`visible_only`) has no consumer yet; it
  starts to matter when scripts can read cookies. A zone using an
  embedder-supplied jar is not vaulted.
