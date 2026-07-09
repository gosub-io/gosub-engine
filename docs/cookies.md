# Architecture: Cookies subsystem

## What this subsystem does

Cookies are how a server asks the browser to remember state: a response carries
`Set-Cookie` headers, the engine stores the cookies it accepts, and every later
request to a matching site carries them back in a single `Cookie` header. The
engine implements this per RFC 6265 / RFC 6265bis with its own parser and
matching logic — it does not delegate cookie handling to the HTTP client.

The subsystem splits into two layers with distinct jobs:

-   **`CookieJar`** — the runtime working set. One jar per zone; it parses
    `Set-Cookie` headers on responses and builds the `Cookie` header for
    requests. Everything security-relevant (domain/path matching, `Secure`,
    `SameSite`, expiry) happens here.
-   **`CookieStore`** — the persistence layer. It creates or restores a jar for
    a zone and writes snapshots back out. Backends: in-memory (ephemeral),
    JSON file, or SQLite. The jar never talks to the store on the hot path;
    persistence is bolted on via a decorator (see below).

Cookies are a **zone service**: the jar/store handles live in `ZoneServices`,
so each zone is its own cookie universe (the profile/container model). Two
zones never see each other's cookies. See
[`zones-and-tabs.md`](zones-and-tabs.md) for how services reach tabs.

## Directory layout

-   `src/engine/cookies/cookies.rs`: type‑erased handles and the serializable `Cookie` struct.
-   `src/engine/cookies/cookie_jar.rs`: `CookieJar` trait, `DefaultCookieJar`, and all parsing/matching/`SameSite` logic.
-   `src/engine/cookies/persistent_cookie_jar.rs`: `PersistentCookieJar` decorator that snapshots to a store after mutations.
-   `src/engine/cookies/store.rs`: `CookieStore` trait.
-   `src/engine/cookies/store/in_memory.rs`: in‑memory store.
-   `src/engine/cookies/store/json.rs`: JSON store.
-   `src/engine/cookies/store/sqlite.rs`: SQLite store (behind the `sqlite_cookie_store` feature; off on WASM).
-   `src/engine/zone/zone.rs`: `Zone` and `ZoneId` integration points.
-   `src/engine/tab/worker.rs`: the fetch-pipeline call sites that read and write the jar.

## The cookie lifecycle

### 1. A response arrives: storing cookies

When a navigation fetch completes, the tab worker calls
`store_response_cookies(final_url, headers, top_level)` on the zone's jar
(`src/engine/tab/worker.rs`). `DefaultCookieJar` then processes every
`Set-Cookie` header:

1.  **Parse** the `name=value` pair and attributes (case-insensitively):
    `Path`, `Domain`, `Expires`, `Max-Age`, `SameSite`, `Secure`, `HttpOnly`.
    Values are kept raw (no URL-decoding); UTF-8 values survive. Empty names
    and malformed pairs are rejected per RFC 6265 §5.2.
2.  **Validate and reject** where the spec requires:
    -   A `Domain` attribute must cover the request host and must not be a
        public suffix — validated against the compiled-in Mozilla Public
        Suffix List (`psl` crate), so a response from `evil.github.io` cannot
        set a cookie for `github.io`.
    -   A `Secure` cookie arriving over plain HTTP is dropped.
    -   The `__Secure-` and `__Host-` name prefixes are enforced
        (RFC 6265bis §4.1.3): `__Host-` additionally requires `Path=/` and no
        `Domain`.
3.  **Resolve expiry**: `Max-Age` wins over `Expires`; both are folded into a
    single `expires` Unix timestamp on the `Cookie` struct. `Max-Age<=0`
    deletes the cookie immediately. No expiry means a session cookie.
4.  **Default the path** from the request URL when no `Path` attribute is given
    (RFC 6265 §5.1.4).
5.  **Store**, bucketed by origin, deduplicating on `(name, domain, path)` with
    last-write-wins while preserving the original creation time (needed for
    RFC 6265bis §5.5 ordering later).

### 2. A request goes out: attaching cookies

Before a navigation request, the worker asks the jar for the header value:

```rust
jar.read().get_request_cookies(&url, Some(&top_level_url), SameSiteContext::SameSite)
```

The jar scans its cookies and applies a filter chain; only cookies that pass
every filter are sent:

-   **Not expired** — expired cookies are skipped (and can be physically
    removed via `purge_expired()`, which stores also run on load).
-   **Domain match** — a cookie with a `Domain` attribute matches that host and
    its subdomains; a host-only cookie (no `Domain`) matches only the exact
    origin that set it.
-   **Path match** — RFC 6265 §5.1.4 prefix rules.
-   **`Secure`** — secure cookies are only sent over HTTPS.
-   **`SameSite`** — enforced against the request's `SameSiteContext` (next
    section).
-   **Third-party policy** — if the jar is configured with
    `ThirdPartyCookiePolicy::Block` or `SameSiteNoneOnly`, cross-site requests
    (registrable domain of the request differs from the top-level page, per
    the PSL) lose some or all cookies.

Survivors are sorted longest-path-first, ties broken by creation time
(RFC 6265bis §5.5), and joined into a single `name=value; name=value` string.

One more protection lives in the network layer rather than the jar: on a
cross-domain redirect, the fetcher (the external `gosub-sonar` crate) strips
the `Cookie` header from the follow-up request so cookies never leak to a
different host.

### 3. Persistence

`DefaultCookieJar` is purely in-memory. Durability comes from wrapping it in a
`PersistentCookieJar`: reads pass through untouched, and after every mutating
call the decorator snapshots the inner jar and hands it to the store via
`persist_zone_from_snapshot(zone, snapshot)`. If a zone is configured with a
`CookieStore` but no explicit jar, the engine builds this wrapper
automatically, and `jar_for(zone)` restores the previous session's cookies at
zone bootstrap.

## Security attributes: what is enforced today

| Attribute | Status |
|---|---|
| `Secure` | Enforced both ways: not **stored** from HTTP responses, not **sent** on HTTP requests. |
| `SameSite` (`Strict`/`Lax`/`None`) | Fully enforced on sending via `SameSiteContext`; a missing attribute defaults to `Lax`; `SameSite=None` requires `Secure`. |
| `HttpOnly` | Parsed and stored, but currently inert — there is no `document.cookie` JS binding yet to hide the cookie from. |
| `__Secure-` / `__Host-` prefixes | Enforced at storage time. |
| `Domain` vs. public suffixes | Enforced via the PSL; supercookies on eTLDs are rejected. |

`SameSiteContext` encodes the request's cross-site situation, and the jar
applies the RFC 6265bis truth table:

| Cookie attribute | `SameSite` | `CrossSiteNavigation` | `CrossSite` |
|---|:---:|:---:|:---:|
| `SameSite=Strict` | ✓ | ✗ | ✗ |
| `SameSite=Lax` | ✓ | ✓ | ✗ |
| *(no attribute)* | ✓ | ✓ | ✗ |
| `SameSite=None; Secure` | ✓ | ✓ | ✓ |

`CrossSiteNavigation` is a cross-site top-level navigation with a safe method
(GET/HEAD); `CrossSite` is a cross-site subrequest or unsafe-method navigation.

## Key types

-   `Cookie` (data model)
    -   Serializable struct for persistence and inspection.
    -   Fields: `name`, `value`, `path`, `domain` (`None` = host-only),
        `secure`, `http_only`, `expires` (Unix seconds; `None` = session
        cookie; `Max-Age` is folded in at parse time), `same_site`,
        `created_at`.
-   `CookieJar` (runtime trait)
    -   `store_response_cookies(url, headers, top_level)`,
        `get_request_cookies(url, top_level, samesite)`, `clear()`,
        `get_all_cookies()` (diagnostics), `remove_cookie(url, name)`,
        `remove_cookies_for_url(url)`, `purge_expired()`.
    -   Accessed via `CookieJarHandle = Arc<RwLock<Box<dyn CookieJar + Send + Sync>>>`.
-   `CookieStore` (persistence/factory trait)
    -   `jar_for(zone)`, `persist_zone_from_snapshot(zone, snap)`,
        `remove_zone(zone)`, `persist_all()`.
    -   Exposed as `CookieStoreHandle = Arc<dyn CookieStore + Send + Sync>`.
-   `ThirdPartyCookiePolicy`: `Allow` (default) / `Block` / `SameSiteNoneOnly`.
-   `SameSiteContext`: `SameSite` / `CrossSiteNavigation` / `CrossSite`.

## Concurrency model

-   `CookieJarHandle`
    -   `RwLock` outside the trait object.
    -   Read lock for queries, write lock for mutations.
-   `CookieStoreHandle`
    -   Only `&self` methods; implementations must be internally synchronized.
    -   Store impls are `Send + Sync` and may use `Mutex`, pools, or transactional back ends.

## Component view

``` mermaid
flowchart TD
  subgraph Zone
    Z["Zone: ZoneId"]
  end

  subgraph Fetch["tab worker (src/engine/tab/worker.rs)"]
    REQ["outgoing request<br>get_request_cookies()"]
    RES["incoming response<br>store_response_cookies()"]
  end

  subgraph Cookie_Runtime["engine/cookies runtime"]
    CJH["CookieJarHandle<br>Arc<RwLock<Box<dyn CookieJar + Send + Sync>>>"]
    PCJ["PersistentCookieJar<br>(decorator, snapshots after writes)"]
    DCJ["DefaultCookieJar<br>origin → Vec<Cookie>"]
    C["Cookie<br>serde Serialize/Deserialize"]
  end

  subgraph Cookie_Persistence["engine/cookies/store"]
    CSH["CookieStoreHandle<br>Arc<dyn CookieStore + Send + Sync>"]
    CST["CookieStore (trait)<br>jar_for(zone)<br>persist_zone_from_snapshot(zone,snap)<br>remove_zone(zone)<br>persist_all()"]
    IM["InMemoryStore"]
    JS["JsonStore"]
    SQ["SqliteStore"]
  end

  Z -->|"cookie_jar()"| CJH
  REQ -->|"read()"| CJH
  RES -->|"write()"| CJH
  CJH --> PCJ
  PCJ --> DCJ
  DCJ -->|"holds"| C

  CSH --> CST
  CST --> IM
  CST --> JS
  CST --> SQ

  CSH -->|"jar_for(zone) at bootstrap"| CJH
  PCJ -. "persist_zone_from_snapshot(zone, snapshot)" .-> CSH
```

## Responsibilities and boundaries

-   Zone owns the `CookieJarHandle` for its lifetime; it does not depend on the store during hot path.
-   The tab worker is the single fetch-pipeline touch point: it reads the jar
    before a navigation request and writes response cookies back afterwards.
-   Store is used at zone bootstrap for `jar_for` and at persistence points for `persist_zone_from_snapshot` or `persist_all`.
-   All jar read/write coordination happens via the handle `RwLock`; stores do not participate in jar locking.
-   Tabs can deviate from their zone via `TabCookieJar` (`Inherit` the zone jar,
    `Ephemeral` private jar, or `Custom` handle) — resolved in
    `src/engine/tab/services.rs`.

## Error handling and durability

-   Store implementations decide durability semantics:
    -   `in_memory`: ephemeral, process‑lifetime only; `persist_*` are no-ops.
    -   `json`: one human‑readable file for all zones; the whole file is
        rewritten per snapshot and writes are not atomic — fine for
        development, not for crash safety.
    -   `sqlite`: transactional durability (DELETE+INSERT per zone snapshot);
        concurrent access via a connection pool; expired cookies are purged on
        load.
-   Trait methods on stores take `&self`; impls must guard internal state.

## Current limitations

Documented so readers don't assume more than the engine does today:

-   **No `document.cookie`.** JavaScript can neither read nor write cookies
    yet; consequently `HttpOnly` is stored but has nothing to hide cookies
    from.
-   **Only top-level navigations carry cookies.** Subresource fetches (images,
    stylesheets, scripts) do not consult the jar, and the single call site
    always passes `SameSiteContext::SameSite` — the `CrossSiteNavigation` /
    `CrossSite` machinery is implemented and tested in the jar but not yet
    driven by the fetch pipeline.
-   **`ThirdPartyCookiePolicy` is not wired up.** Jars are created with the
    default `Allow`; no production code path selects `Block` or
    `SameSiteNoneOnly` yet.
-   **`EngineConfig.cookie_jar_partitioning`** (`CookiePartitioning`) is
    declared in the config but not yet consumed by the cookie subsystem.

## Extension points

-   New jar implementation
    -   Implement `CookieJar + Send + Sync`.
    -   Wrap with `CookieJarHandle::new(your_impl)`.
    -   Note: `PersistentCookieJar` snapshots by downcasting to
        `DefaultCookieJar`; a custom jar needs its own persistence strategy.
-   New store implementation
    -   Implement `CookieStore + Send + Sync` with internal synchronization.
    -   Decide snapshot format; use `DefaultCookieJar` as the interchange.
