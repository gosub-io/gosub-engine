# Fonts

Text handling is split into two parts:

1.  **Font systems** implement the `FontSystem` trait and live in `gosub_fontmanager`. They register fonts, resolve CSS font queries to concrete fonts (including their raw bytes), shape text into positioned glyph runs, and measure text so the layouter can size boxes. They do not draw.
2.  **Glyph painters**, one per render backend, paint the `ShapedText` glyph runs carried on text paint commands using the backend's native glyph call. They do not shape.

The font system is a type parameter on the config. Measurement, shaping, and painting all go through that one instance, so a layout box is always sized with the same font that paints it.

## Font systems (`FontSystem`)

The trait lives in [`gosub_interface/src/font_system.rs`](../crates/gosub_interface/src/font_system.rs):

``` rust
pub trait FontSystem: Send + Sync + 'static {
    /// Register a font from raw bytes (`@font-face` web fonts, bundled fallbacks).
    fn register_font(&mut self, data: Vec<u8>, family_override: Option<&str>) -> Result<(), FontError>;
    /// Resolve a CSS font query to a concrete font, including its raw bytes.
    fn resolve(&mut self, query: &FontQuery<'_>) -> Result<ResolvedFont, FontError>;
    /// Every family resolvable by name (system fonts + registered fonts), sorted and deduped.
    fn families(&mut self) -> Vec<String>;
    /// Shape `text` laid out in `style` into positioned glyph runs.
    fn shape(&mut self, text: &str, style: &TextStyle) -> ShapedText;
    /// Measure the bounding box of `text` laid out in `style`, in CSS pixels.
    /// Provided: shapes and reads the bounding box; implementations may override.
    fn measure(&mut self, text: &str, style: &TextStyle) -> (f32, f32) { … }
    /// The confinement tier, knowable without an instance (see below).
    /// Provided: answers `Confinement::Full`.
    fn confinement() -> Confinement where Self: Sized { … }
    /// Load everything that needs the filesystem now, then answer how confined
    /// a renderer process using this font system may be (see below).
    /// Provided: warms every family and answers `Confinement::Full`.
    fn prepare_for_confinement(&mut self) -> Confinement { … }
}
```

Each returned `ShapedRun` names the font (bytes included) that was actually used for its glyphs, including mid-string fallback. `families()` lists every family resolvable by name — the same database `resolve` matches against — for consumers like a font-picker UI or the Local Font Access API; generic CSS keywords such as `sans-serif` are resolution aliases and are not listed. Painting a `ShapedText` is the render backend's job, not the font system's.

The trait file also defines the shared value types: `TextStyle` (family, size, weight, style, stretch, optional line height and wrap width, letter spacing, display scale), `FontQuery` / `ResolvedFont` (family resolution with raw `FontBlob` bytes), and `ShapedText` / `ShapedRun` / `ShapedGlyph` (positioned glyph runs).

### Implementations

| Implementation     | Crate / file                                                                                 | Backed by                                  | Notes |
|--------------------|----------------------------------------------------------------------------------------------|--------------------------------------------|-------|
| `ParleyFontSystem` | [`gosub_fontmanager/src/parley_system.rs`](../crates/gosub_fontmanager/src/parley_system.rs) | Parley + Fontique                          | The default; portable, not tied to a renderer. |
| `CosmicFontSystem` | [`gosub_fontmanager/src/cosmic_system.rs`](../crates/gosub_fontmanager/src/cosmic_system.rs) | cosmic-text, fontdb, rustybuzz, swash      | Pure-Rust alternative to Parley; not used by any config by default. |
| `PangoFontSystem`  | [`gosub_fontmanager/src/pango_system.rs`](../crates/gosub_fontmanager/src/pango_system.rs) (feature `pango`) | Pango / fontconfig                         | `resolve` queries fontconfig directly (the same database Pango picks from); `shape` exports the `PangoLayout` glyph runs. Registers web fonts into the process-global fontconfig config. |
| `SkiaFontSystem`   | [`gosub_fontmanager/src/skia_system.rs`](../crates/gosub_fontmanager/src/skia_system.rs) (feature `skia`)     | Skia, `skia_safe`, paragraph layout        | Measures and shapes through a thread-local `FontCollection`; `resolve`/`shape` export font bytes via `Typeface::to_font_data`. |

The heavyweight engines are feature-gated (`pango` pulls in the GTK/fontconfig stack, `skia` pulls in `skia-safe`). The Cairo and Skia renderer crates enable their feature and re-export the type for convenience (`gosub_renderer_cairo::PangoFontSystem`, `gosub_renderer_skia::SkiaFontSystem`).

### Font systems under process isolation

A future renderer process runs behind a default-deny seccomp sandbox that wants to forbid file access outright — but font stacks read font files, and *when* they read them differs per stack. Because the font system is a configuration choice, "can a renderer be sandboxed" is not a fact about the engine; it is a fact about each font system, and only the implementation knows what it defers to the filesystem. That is what `FontSystem::prepare_for_confinement()` expresses: the implementation loads whatever it can front-load, then answers with a `Confinement` value naming the strongest sandbox tier it supports. All of the below was measured on Linux via the `isolation-harness` font scenarios (`fonts-under-lockdown`, `webfont-under-lockdown`, and the `…-font-readable-…` variants, each taking the backend name as an argument).

| Answer | Sandbox the renderer gets | Who |
|---|---|---|
| `Confinement::Full` | `lock_down_renderer()`: **no file access at all** | Parley, cosmic-text |
| `Confinement::FontPathsReadable` | `lock_down_renderer_with_font_access()`: read-only font paths + one private writable scratch, nothing else | Pango, Skia |
| `Confinement::Unsupported(reason)` | none — the engine must fall back to single-process rendering | no bundled system |

Per implementation:

- **Parley (fontique)** — defers file reads lazily **per face**: a family-level warm-up loads only the face the default attributes select (regular), and the first *bold* heading laid out under the sandbox died opening `…-Bold.ttf`. Its override loads every face of every family — into **both** of the system's source caches, which are separate (`resolve` reads its own; parley's shaping reads the one inside `FontContext`) — at ~110 ms and near-zero RSS, since fontique memory-maps the files. Fully confinable.
- **cosmic-text** — defers lazily **per face** (through its `get_font` cache), and shaping consults fallback faces that a family-by-family warm-up never touches, so the trait default is *not* enough (measured as a `SIGSYS` on `openat` mid-shape). Its override loads every face in the fontdb instead: ~20 ms / +46 MiB, and fully confinable. Note that even a web font delivered as bytes only shapes safely because preparation ran — shaping it still consults fallback faces.
- **Pango** — cannot be fully confined, and no warm-up changes that: fontconfig re-validates its caches against the filesystem (`access(2)`, then re-opening files) *while matching*, in steady state. It answers `FontPathsReadable` and does no preparation work at all — under that tier it shapes cold, with nothing pre-loaded. Two Pango-specific limitations to know about: web fonts must be **staged as temp files** (fontconfig's app-font API takes a path; there is no from-memory variant), which is why the tier includes a writable scratch directory with `TMPDIR` pointed at it; and the fontconfig config is **process-global**, so registered web fonts are visible engine-wide rather than per-instance.
- **Skia** — same fontconfig story on Linux (its default `FontMgr` is fontconfig-backed), so the same `FontPathsReadable` answer. Its font machinery additionally wants `getcwd`, `fstatfs`/`statfs`, and `fadvise64`, all included in the tier's allowlist. Its `FontCollection` is **thread-local**, which under full confinement would be an independent problem (a worker thread rebuilds its collection from scratch, re-reading files); under the font-readable tier it is harmless, since the paths stay reachable.

The font-readable tier is the WebKitGTK arrangement (their fontconfig-based renderers get font directories bind-mounted read-only) and lives in `gosub_sandbox`: the renderer seccomp baseline plus the file-reading syscalls, with a **Landlock** ruleset confining those syscalls to `font_filesystem_paths()` — the font directories, fontconfig configuration, and caches, read-only. What an exploited renderer gains under it, compared to full confinement, is the ability to read world-readable font data and enumerate installed fonts; network, exec, devices, and all other filesystem access stay denied. On kernels without Landlock the path scoping degrades (the syscalls stay allowed unscoped), which is one more reason the engine applies the strongest tier the configured font system answers rather than defaulting to the relaxed one.

The tier also decides how renderer processes are *created* (`gosub_engine::fork_server`). A `Full` system gets a **fork server**: a process that builds and prepares the font system once, confines itself, and forks renderers that inherit the warmed state copy-on-write — warm-up paid once, free per renderer. A `FontPathsReadable` system gets **no fork server**: warming buys nothing when the files stay reachable (renderers are exec'd fresh, ~3.7 ms), and the fork server deliberately never constructs such a stack — which is why `Confinement` is answerable *statically* via `FontSystem::confinement()`. That is not an optimization but a hard constraint, measured with Pango: GLib insists on spawning a worker thread during setup, a process that has unshared its PID namespace (as the fork server has) cannot create threads, and GLib escalates the failure to a fatal abort. Reaching the fork-server role requires `child_process::dispatch_with::<AppConfig>()` — plain `dispatch()` is type-erased and cannot construct the embedder's configuration.

A forked renderer runs the actual pipeline (`fork_server::renderer::render_page`): parse, style, layout, layering, tiling, paint — single-threaded (the renderer filter has no `clone`), shaping through the inherited fonts. Fonts are not the only lazily-file-loading state that had to move pre-fork: constructing a `MediaStore` decodes the placeholder SVG, whose decoder loads a system fontdb from disk, so the fork server builds the store once before its lockdown and renderers inherit it (`TaffyLayouter::with_font_system_and_media_store` exists so the layouter constructs neither).

Stage 6 runs there too when the configuration provides a rasterizer via `RenderConfiguration::forked_tile_rasterizer()` (default `None`) — a seam separate from `RenderBackend::create_rasterizer` because a re-exec'd child has only *types* to construct from, never a backend instance. The rasterized tiles come back **zero-copy** over `gosub_ipc::shm`: sealed memfds passed as fds, mapped and validated broker-side. `CairoRasterizer` (with `gosub_renderer_cairo`'s default GTK/Pango features off) rasterizes under the strictest sandbox and pairs with any font system; the engine feature `cairo-tiles` compiles it into the isolation harness. Subresource loads from the renderer come later still.

### How a font system reaches layout and rendering

A single instance is shared as `Arc<Mutex<dyn FontSystem>>` between the layouter and the rasterizer:

-   Your config implements `HasFontSystem` (usually via `DefaultRenderConfig<Backend, FontSystem>`, see [configuration.md](configuration.md)), which hands the `Arc` to both sides.
-   In the render pipeline, `Rasterable::font_system()` ([`gosub_render_pipeline/src/rasterizer.rs`](../crates/gosub_render_pipeline/src/rasterizer.rs)) exposes the rasterizer's font system so the layouter can adopt the same instance. It returns `None` for rasterizers that don't shape through a `FontSystem` (e.g. the null rasterizer); the layouter then falls back to its own `ParleyFontSystem` (`TaffyLayouter::new()` in [`gosub_render_pipeline/src/layouter/taffy.rs`](../crates/gosub_render_pipeline/src/layouter/taffy.rs)).
-   `register_font` is how `@font-face` web fonts and bundled fallbacks (Roboto, from `gosub_shared`) enter the collection, once, visible to both measurement and drawing.

Measurement happens in CSS pixels; DPI scaling is applied later in the pipeline.

## Text painting

Text is shaped once, at paint-command build time: the pipeline `Painter` calls `FontSystem::shape(...)` on the configured font system (the same instance the layouter measured with) and stores the resulting `ShapedText` on the `Text` paint command. Each renderer paints those runs with its native glyph call — vello via `draw_glyphs`, Skia via `TextBlobBuilder`, cairo via FreeType faces + `cairo_show_glyphs` (each in `src/rasterizer/text/glyphs.rs`).

The contract between shaping and painting is raw font bytes plus glyph IDs and positions, so any font system works with any backend; there is no pairing matrix. Shaping honours `TextStyle::align`, and each `ShapedRun` carries underline/strikethrough metrics for decorations. Colour emoji works on cairo through FreeType's colour-bitmap support.

The usual pairings follow the platform stack: `PangoFontSystem` with Cairo (GTK desktop), `ParleyFontSystem` with Vello, `SkiaFontSystem` with Skia (e.g. `bin/gosub-screenshot` uses `DefaultRenderConfig<SkiaBackend, SkiaFontSystem>`), but any combination is valid.

### Implementation notes

-   **Pango** measures with its own natural line height, matching how it lays out lines when shaping; `TextStyle::line_height` is not applied there.
-   **Skia** applies the CSS line height during measurement (as a multiple of font size), matching its shaping. It also prunes the CSS family list so an unavailable leading family can't capture the platform default.
-   **Pango's `system-ui`** is resolved via GSettings, which must happen on the GTK main thread before background rendering starts (`init_from_gtk_thread`; the process-wide singleton in `pango_system` exists for this).
