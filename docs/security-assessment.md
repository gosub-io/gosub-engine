# Security assessment of the process-isolation design

What an attacker in each position of the design can reach, assessed against
the code as of the `stack/19-security` branch, and what was changed because of
it. [process-isolation.md](process-isolation.md) describes the design; this
page is the adversarial reading of it. Severity: critical is cross-site data or
the host; high is local files or the broker; medium is denial of service of the
broker or another tab, or deceiving the user; low is the rest.

The assessment was done per attacker position, each with the assets it would
go for, and every finding below was verified in the code. Fixed items name the
commit's subject on this branch.

## Positions and assets

| Position | Holds | Goes for |
|---|---|---|
| A. compromised renderer | code execution in a renderer process | other sites' data, local files, the broker, siblings, the host, the user's trust in the UI |
| B. compromised service child | the network process, the vault, a decoder, the storage service, the fork server | the broker, other children, the host |
| C. local attacker | another process as the same user, another user, a remote page reaching 127.0.0.1 | browsing data, the on-disk stores, the children |
| D. hostile page | ordinary web content, no exploit | what still runs in the broker, other sites' cookies, the private network |

Position A assumes the renderer tier runs, and it runs only where the
embedder supplies a forked tile rasterizer and uses Parley or cosmic fonts:
the engine's own `DefaultRenderConfig` has no rasterizer without the
non-default `cairo-tiles` or `skia-tiles` features. Everywhere else HTML,
CSS, layout, paint, web fonts and inline SVG run in the embedder's process,
so a parser bug is position D against the embedder, not position A against
a renderer.

## Fixed on this branch

- **The network process's Landlock covered only its idle main thread**
  (critical: every zone's persisted cookies and localStorage, and anything
  the user can read, from the process that parses every response). Landlock
  binds the thread that applies it and the threads created after it, and
  the tokio runtime was built before the lockdown, so its workers - where
  every request runs - were never scoped; the escape audit ran on the main
  thread and reported the scope active. Fixed in "network process: scope
  its files before the runtime starts its workers": the filesystem half
  (`scope_net_filesystem`: Landlock plus read-only opens at the syscall
  layer) is applied while the process is still one thread, the runtime is
  built after it, the seccomp half follows with TSYNC, and the audit runs
  on a runtime worker. The `net-thread-landlock` probe pins the property:
  a thread started after the scope is refused a file outside it, and one
  started before it still reads the file, so the order is what matters.
- **A fork server's claimed pid moved any process into a renderer cgroup**
  (high, broker DoS). `RendererSpawned { pid }` was taken on the fork server's
  word and written into a 1.25 GiB, 256-task cgroup; pid 0 is the writer
  itself. The broker now opens a pidfd for the pid first and requires `/proc`
  to show the fork server as its parent; a wrong claim is a hostile fork
  server and stops it. The pidfd is kept.
- **A hostile renderer could not be ended** (medium). The per-request deadline
  bounds a page that loops, not a renderer that disarmed it (`setitimer` and
  `rt_sigaction` are on its allowlist). The broker now kills a renderer it has
  given up on through the pidfd, whatever the process is doing.
- **The telemetry server answered any Host** (high when `telemetry.metrics_enabled`
  is on). A remote page whose DNS answer switches to 127.0.0.1 after it loads
  reached `/events`, the firehose of every URL fetched, same-origin; and
  `POST /metrics/reset` was a cross-site form away. Requests whose `Host` is
  not a loopback name are refused, which closes the read. A cross-site form
  reaches the reset with the server's name as `Host`, and gosub's own forms
  send no `Origin`, so the reset requires an `X-Gosub-Reset` header no page can
  send (a form cannot set one; a script needs a CORS preflight the server never
  answers), and an `Origin`, when sent, equal to the server's own host and port.
- **A page could log the user out of every other site in the zone** (medium).
  The jar-wide cookie cap evicted the oldest cookie anywhere; a page naming
  3000 of its own subdomains was the newest. Eviction now takes from the
  registrable domain holding the most cookies: the origin being written when
  it sits in that domain (RFC 6265 §5.3 step 12), else that domain's oldest.
  Never the writer's domain merely because it wrote; at a cap a flooder
  filled, a victim's own write would otherwise evict the victim's session.
  The jar is also bounded in bytes, every string a cookie carries counted.
- **A local page could have the broker read a device file without end**
  (high, DoS; also on the in-process path). `<img src="file:///dev/zero">`
  from any local HTML file. The file loader serves regular files, up to
  256 MiB read with a bound, and a directory as a generated listing; a device
  or a FIFO is refused. The type is checked on the path before the `open`
  and on the opened file after it, and the open does not block, so a FIFO
  or device swapped in between is opened, refused and closed without a
  read.
- **The broker's own lockdown was never applied** (medium). `lock_down_broker`
  existed in the sandbox crate with no caller in the engine or any example;
  every statement about the broker's Landlock scope held for nobody. It is
  part of the embedder contract now, takes the embedder's writable paths,
  the mini-browser applies it, and the harness runs every engine scenario
  under it.
- **The broker's deny-list did not cover the x32 ABI** (medium, on kernels
  built with `CONFIG_X86_X32_ABI`). A deny-list names syscall numbers, and on
  x86_64 each has a twin with `__X32_SYSCALL_BIT` set that the filter sees
  under the same architecture, so `ptrace | 0x4000_0000` passed the broker's
  `Allow` default. A stacked pre-filter now traps the whole x32 range; the
  `broker-seccomp-x32` probe pins it. The children's allowlists never had
  the gap: an x32 number is on no list.
- **On-disk stores used the umask** (low-medium). localStorage directories
  and the JSON cookie store are created 0700/0600; the SQLite store's
  journal lives in the 0700 directory.
- **A page's evictions scanned the broker's tile memory** (low-medium, CPU on
  the tab thread). Removal is a lookup now.
- **A renderer's title reached the window with control and bidi characters**;
  **a hit region's image URL reached the embedder's menus unchecked** (medium,
  UI). Both bounded in the broker.
- **The link handed over for a renderer was only checked to be a socket**
  (low). It must be a stream socket.
- **`Set-Cookie` values and URL credentials reached the embedder** (low).
  A request's `Cookie` was redacted in resource events and a response's
  `Set-Cookie` was not, and a reported URL kept its `user:password@`. Both
  are redacted now, under the same switch as `Cookie`
  (`set_send_sensitive_headers`). Under the network process with the vault,
  cookie values no longer reach the broker either: the child drops
  `Set-Cookie` from a reply once the vault has stored it, and redacts both
  cookie headers in the events it relays. One the vault did not take stays on
  the reply, and the broker stores it, so it is not lost. URL credentials
  still cross the link in event URLs and are redacted where the broker
  reports them.

## Accepted, with what the sandbox still prevents

- **The network process is trusted engine code.** A compromised one sees
  every request and response in clear, attaches cookies (with the vault, a
  `Get` and a `Store` per redirect hop at any web URL, 64 each per ticket, so
  it can set cookies for other hosts of a zone with a request in flight, and
  read another site's cookies by claiming a redirect there: only those that
  site's cookie policy sends on a cross-site request, never `Strict` ones,
  and a hop no reported redirect explains gets the process killed), forges
  responses, and enforces the private-network policy itself, so it can reach
  127.0.0.1 and the LAN with the host network namespace it keeps. Its sandbox
  still denies: files beyond the read-only resolver and CA paths (it refuses
  to start without Landlock), unix and netlink sockets (no D-Bus, no agents),
  exec, devices, forks into namespaces, ptrace.
- **A renderer of a `file:` page reads any readable local file through the
  broker and can send it out.** A `file:` document may embed `file:`
  subresources, as it may in Chromium and Firefox (Firefox's strict file
  origin policy restricts script reads, not embedding), and local
  documentation (`../static.files`) depends on it. With a renderer exploit
  that embedding is a read, and any HTTP request is a channel out. Mitigation
  is `net.file.enabled` off where local pages are not needed; a per-directory
  bound would break rustdoc-style output and was not taken.
- **A gopher page on this machine or the local network can aim a line of
  text at any port in its space.** A gopher request is the URL's selector
  written to a socket, so a link is a way to send one line to whatever
  listens there. A page may only navigate to gopher from gopher, never load
  it as a subresource, and a link or form reaches no further than its page's
  space (a public menu cannot point at 127.0.0.1); the Fetch standard's
  bad-port list applies, and a CR, LF, TAB or NUL in the URL never reaches the
  line. What is left is a menu the user opened on loopback or the LAN
  pointing at a neighbour that takes single-line commands. Mitigation is
  `net.gopher.enabled` off where gopher is not needed. Gopher is also
  plaintext: anything on the path can change a page.
- **Finger is plaintext, and a finger server decides what it tells.** A
  finger answer is shown as text with no links, and no page may navigate to
  finger (only a page on the same protocol may, and a finger page links
  nowhere), so a finger request is one the user typed. Ports follow the same
  rule as gopher's, with finger's own port 79 open to finger only, and a
  control byte in the URL never reaches the line. A query naming another
  host (`user@host`) is refused: RFC 1288 forwarding has the server connect
  onward, past any reach the engine judged. Terminal escape sequences in an
  answer are dropped before it is shown. Mitigation is `net.finger.enabled`
  off where finger is not needed.
- **Gemini trusts the first certificate a server shows.** Gemini servers
  mostly sign their own certificates, so no authority is asked: the first
  visit to a host and port accepts whatever certificate comes, and later
  visits must show the same one or fail before the request is sent. A first
  visit on a hostile network can therefore be intercepted, and the address
  bar shows the same padlock as for an https page an authority vouched for.
  The certificate's name and dates are not checked; the pin stands in for
  both. Pins live in the zone, in memory unless the embedder passes a store
  (`ZoneBuilder::known_hosts`, `SqliteKnownHosts`); with the default every
  restart is a first visit again. A changed certificate fails with the new
  one's fingerprint, and the embedder may let the user trust exactly that
  one; a user who clicks through a real interception has pinned the
  interceptor. With the network process on, that fingerprint comes from the
  network process, which a compromised one could choose, but such a process
  already sees and can change every response. A redirect to another gemini
  URL is followed, at most five times in all, and every server on the way is
  held to the zone's pin for it, or on a first visit to the certificate it
  showed first in that request. Only the server the page was asked for gets a
  changed certificate offered for trusting. With the network process on, it
  asks the broker for each other server's pin, for at most five servers per
  request, and the broker records a first visit's certificate only for a
  server the request asked about; a compromised network process can still
  make up a redirect to learn a pin, which is no secret, or have a first
  visit pinned to a certificate of its choosing, which it could serve anyway.
  A redirect to another scheme is shown as a link. A server answering `44`,
  slow down, is left alone for a delay that starts at a second and doubles
  each time it says so again; the delays are kept per process, for all
  zones together, so one zone can tell another was told to slow down by the
  same server. Input a server asks for goes
  into the URL's query, so a status `11` (sensitive) answer such as a
  password lands in history and in the server's logs, as with any gemini
  client. Many servers close without TLS `close_notify`, so a response cut
  short on the path reads as complete. The request line is the URL alone and
  goes out only after a handshake, so a plaintext service on a port a link
  can reach gets a TLS hello it rejects, not a command. Links and forms follow
  the gopher rules: navigations only, to gemini or the web, within the page's
  space. Mitigation is `net.gemini.enabled` off where gemini is not needed.
- **Cross-origin authenticated bytes of embeddable types reach the renderer.**
  No-CORS subresource loads carry the zone's cookies for the URL (`SameSite`
  judged against the tab, so `SameSite=None` ones cross sites), and opaque
  response blocking lets images, media, CSS, scripts, fonts and sniffed
  unknown types through, as the ORB algorithm does. It lets more through
  than the algorithm: image, video, audio and font types pass on their label
  where ORB sniffs them, and a response sniffed as none of those is not put
  through ORB's final step, parsing it as JavaScript. A compromised renderer
  reads them. CORS and credentials mode on `NeedResource` are the fix, not here.
- **Every renderer shares the fork server's address-space layout**, being
  forked from it; an address leak in one is the layout of all, and of the fork
  server, which also deserialises a one-shot renderer's frames when relaying
  them (that path is live only when the resident pool failed to start). The
  zygote trade-off, as in Chromium.
- **A thread the fork server's font system starts during warm-up is not
  Landlock-scoped.** Landlock binds the thread that applies it and the threads
  created after it, and the fork server can only choose its tier (and so its
  paths) once the font system is built and has said what it needs. A GLib or
  Skia worker started during that build keeps an unscoped filesystem in the
  fork server, read-only through the opens pre-filter and inside its seccomp
  filter. It never reaches a renderer: `fork` copies only the calling thread,
  which is scoped, and the fork server parses broker messages, not page
  content. Scoping before the build would need a layered second ruleset and a
  path list for every backend's warm-up, which was not taken.
- **Favicon bytes are handed to the embedder undecoded** (`FavIconChanged`),
  so an embedder that decodes them does so in the broker. The event's
  documentation says so; the mini-browser ignores it.
- **All host-less documents of a zone (`data:`, `LoadHtml` with a host-less
  base) share one renderer.** An embedder showing its own content that way
  shares a process with page-controlled `data:` documents; key such content
  on a base URL with a host.
- **Input to a remote page is handled by the renderer, and its answers are
  requests.** Every press, key and text goes to the process that holds the
  page's DOM; what comes back (focus, cursor, a navigation, a picker, the
  clipboard, a pointer capture) is judged in the broker before anything
  happens: a navigation under the rule a link gets, a cursor only from a
  pointer pass, the clipboard only from the chord that asks for it, bounds
  clamped to the viewport, strings and counts bounded like hit regions with
  an over-cap frame treated as a crash. A compromised renderer can still
  misdirect the page it holds - type into the wrong field, submit a form it
  composed to an origin the page may reach, pick how much of the page's URL a
  navigation sends as `Referer`, show a cursor - which is what
  holding the page means; it cannot reach the clipboard without the user's
  chord, navigate to a scheme a page may not, or make the broker act on a
  frame it did not bound. The effect rules are unit-tested and a harness
  scenario plays a lying renderer over a socket pair.
- **The escape audit is the child's own report.** It proves the spawn path
  confines a child; a compromised child reports clean. It is a test, not an
  attestation.
- **The telemetry server is readable by any local user** when enabled (a
  unix socket or a per-launch token would close that); it is off by default.

## Open

- A test that forces an out-of-process render to fail after the renderer
  tier started and asserts the tab stayed blank (the "never in the broker"
  guarantee) needs a hook the engine does not have.
- The Pango and Skia tier runs under no CI test; `pids.max` is written and
  never read back by a probe.
- The broker's timing table grows per navigation in any embedder without
  metrics on (pre-existing on main).
- A renderer that answers an input pass with an over-cap frame is ended by
  the exchange and replaced by the pool like any crash; that replacement is
  tested for a crashed renderer, not yet for a lying one at engine level.
- The 5 s input deadline in the renderer has no test: nothing deterministic
  makes a pass that slow, and a renderer that never answers only hits the
  broker's 60 s reply timeout.
