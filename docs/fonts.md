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
