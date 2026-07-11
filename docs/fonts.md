# Fonts

Text handling in Gosub has two halves with a single seam between them:

1.  **Font systems** --- implement the `FontSystem` trait (all in `gosub_fontmanager`). They register fonts, *resolve* CSS font queries to concrete fonts (with their raw bytes), *shape* text into positioned glyph runs, and *measure* it so the layouter can size boxes. They never draw anything.
2.  **Glyph painters** --- one per render backend. They paint the `ShapedText` glyph runs carried on text paint commands, using the backend's native glyph call. They never shape anything.

You pick a font system at runtime (it's a type parameter on your config); every backend paints whatever it shaped. Measurement, shaping, and painting all flow through the one configured instance, so a layout box is never sized with one font and painted with another --- consistency holds by construction, not convention.

## Family 1: font systems (`FontSystem`)

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
}
```

Every implementation exposes the full lookup → shape → measure pipeline through the trait; each returned `ShapedRun` names the font (bytes included) that was *actually* used for its glyphs, mid-string fallback included. `families()` enumerates the installed inventory (plus registered web fonts) — the same database `resolve` matches against — for consumers like a font-picker UI or the Local Font Access API; generic CSS keywords are resolution aliases, not families, so they are not listed. Drawing is the one job that stays outside: painting a `ShapedText` is the render backend's business. The trait is the *only* interface between font systems and backends — there is no downcast escape hatch, so any font system works with any backend by construction.

The trait file also defines the shared value types: `TextStyle` (family, size, weight, style, stretch, optional line-height and wrap width, letter spacing, display scale), `FontQuery` / `ResolvedFont` (family resolution with raw `FontBlob` bytes), and `ShapedText` / `ShapedRun` / `ShapedGlyph` (positioned glyph runs).

### Implementations

| Implementation     | Crate / file                                                                                 | Backed by                                  | Notes |
|--------------------|----------------------------------------------------------------------------------------------|--------------------------------------------|-------|
| `ParleyFontSystem` | [`gosub_fontmanager/src/parley_system.rs`](../crates/gosub_fontmanager/src/parley_system.rs) | Parley + Fontique                          | The default; portable, not tied to a renderer. |
| `CosmicFontSystem` | [`gosub_fontmanager/src/cosmic_system.rs`](../crates/gosub_fontmanager/src/cosmic_system.rs) | cosmic-text, fontdb, rustybuzz, swash      | Deliberately maintained pure-Rust alternative to Parley; no config uses it by default. |
| `PangoFontSystem`  | [`gosub_fontmanager/src/pango_system.rs`](../crates/gosub_fontmanager/src/pango_system.rs) (feature `pango`) | Pango / fontconfig                         | `resolve` queries fontconfig directly (the same database Pango picks from); `shape` exports the `PangoLayout` glyph runs. Registers web fonts into process-global fontconfig. Requires one-time init from the GTK main thread, `init_from_gtk_thread`, to resolve `system-ui`; a process-wide singleton exists for this reason. |
| `SkiaFontSystem`   | [`gosub_fontmanager/src/skia_system.rs`](../crates/gosub_fontmanager/src/skia_system.rs) (feature `skia`)     | Skia, `skia_safe`, paragraph layout        | Measures and shapes through a thread-local `FontCollection`; `resolve`/`shape` export font bytes via `Typeface::to_font_data`. |

All four implementations live in `gosub_fontmanager` — a font system is renderer-independent by construction, so the crate is the single home. The heavyweight engines are feature-gated (`pango` pulls the GTK/fontconfig stack, `skia` pulls `skia-safe`); the Cairo and Skia renderer crates enable their feature and re-export the type for convenience (`gosub_renderer_cairo::PangoFontSystem`, `gosub_renderer_skia::SkiaFontSystem`).

### How a font system reaches layout and rendering

A single instance is shared as `Arc<Mutex<dyn FontSystem>>` between the layouter and the rasterizer:

-   Your config implements `HasFontSystem` (usually via `DefaultRenderConfig<Backend, FontSystem>`, see [configuration.md](configuration.md)), which hands the `Arc` to both sides.
-   In the render pipeline, `Rasterable::font_system()` ([`gosub_render_pipeline/src/rasterizer.rs`](../crates/gosub_render_pipeline/src/rasterizer.rs)) exposes the rasterizer's font system so the layouter can adopt the same instance. It returns `None` for rasterizers that don't shape through a `FontSystem` (e.g. the null rasterizer); the layouter then falls back to its own `ParleyFontSystem` (`TaffyLayouter::new()` in [`gosub_render_pipeline/src/layouter/taffy.rs`](../crates/gosub_render_pipeline/src/layouter/taffy.rs)).
-   `register_font` is how `@font-face` web fonts and bundled fallbacks (Roboto, from `gosub_shared`) enter the collection --- once, visible to both measurement and drawing.

Measurement happens in **CSS pixels**; DPI scaling is applied later in the pipeline.

## Family 2: text painting

Painting is glyph-based everywhere and needs no font engine at all. Text is shaped **once, at
paint-command build time**: the pipeline `Painter` calls `FontSystem::shape(...)` on the
configured font system (the same instance the layouter measured with) and stores the resulting
`ShapedText` on the `Text` paint command. Each renderer then just paints those runs with its
native glyph call — vello via `draw_glyphs`, Skia via `TextBlobBuilder`, cairo via FreeType
faces + `cairo_show_glyphs` (each in `src/rasterizer/text/glyphs.rs`).

Because the contract between shaping and painting is raw font bytes + glyph IDs, **any font
system works with any backend** — there is no pairing matrix and no `text_*` feature selection
anymore. Alignment and underline/strikethrough are honoured (shaping carries `TextStyle::align`;
each `ShapedRun` carries decoration metrics). Colour emoji work on cairo via its FreeType
colour-bitmap support (verified with Noto Color Emoji).

Historical note: each backend used to drive its own text engine natively (pangocairo, Parley,
Skia textlayout) behind `text_*` cargo features, with measurement and drawing kept consistent by
convention. Those paths were deleted once the glyph painters were validated pixel-for-pixel
against them.

## Which font system pairs with which renderer

Any of them — measurement, shaping, and painting all flow through the one configured
`FontSystem` instance, so consistency holds by construction. The conventional defaults match the
platform stack: `PangoFontSystem` with Cairo (GTK desktop), `ParleyFontSystem` with Vello,
`SkiaFontSystem` with Skia (e.g. `bin/gosub-screenshot` uses
`DefaultRenderConfig<SkiaBackend, SkiaFontSystem>`), but mixing is now valid.

### Per-implementation quirks worth knowing

-   **Pango** uses its own natural line height during measurement, deliberately matching how the Cairo rasterizer draws; `TextStyle::line_height` is not applied there.
-   **Skia** *does* apply the CSS line height during measurement (as a multiple of font size), exactly like its draw path --- skipping this used to make measured boxes shorter than the painted text. It also prunes the CSS family list so an unavailable leading family can't capture the platform default.
-   **Pango's `system-ui`** resolution must happen on the GTK main thread before background rendering starts.
