# Fonts: the two backend families

Text handling in Gosub is split across **two independent backend families** that are easy to
confuse, because both come in `pango`, `parley`, and `skia` flavours:

1. **Font systems** — implement the `FontSystem` trait. They register fonts and *measure*
   text so the layouter can size boxes. They never draw anything.
2. **Text rasterizers** — the code that actually *renders glyphs* onto a surface. Selected
   per render-backend crate at compile time with `text_*` cargo features. They have no
   shared trait; each exposes a `do_paint_text(...)` function.

You pick a font system at runtime (it's a type parameter on your config); you pick a text
rasterizer at build time (it's a cargo feature on the renderer crate). The two must agree —
the whole design exists to guarantee that text is **measured and drawn against the same font
collection**, so a layout box is never sized with one font and painted with another.

## Family 1: font systems (`FontSystem`)

The trait lives in [`gosub_interface/src/font_system.rs`](../crates/gosub_interface/src/font_system.rs)
and is deliberately small:

```rust
pub trait FontSystem: Send + Sync + 'static {
    /// Register a font from raw bytes (`@font-face` web fonts, bundled fallbacks).
    fn register_font(&mut self, data: Vec<u8>, family_override: Option<&str>) -> Result<(), FontError>;
    /// Measure the bounding box of `text` laid out in `style`, in CSS pixels.
    fn measure(&mut self, text: &str, style: &TextStyle) -> (f32, f32);
    /// Recover the concrete type so a render backend can call its native shaping/draw path.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
```

Shaping and drawing are intentionally **not** on the trait: the shaped representation and the
draw target are backend-specific. A render backend that wants more than measurement downcasts
via `as_any_mut` and calls the concrete type's own inherent methods.

The trait file also defines the shared value types: `TextStyle` (family, size, weight, style,
stretch, optional line-height and wrap width, display scale), `FontQuery` / `ResolvedFont`
(family resolution with raw `FontBlob` bytes), and `ShapedText` / `ShapedRun` / `ShapedGlyph`
(positioned glyph runs, produced by implementations that shape natively).

### Implementations

| Implementation | Crate / file | Backed by | Notes |
|---|---|---|---|
| `ParleyFontSystem` | [`gosub_fontmanager/src/parley_system.rs`](../crates/gosub_fontmanager/src/parley_system.rs) | Parley + Fontique | The default; portable (not tied to a renderer). |
| `CosmicFontSystem` | [`gosub_fontmanager/src/cosmic_system.rs`](../crates/gosub_fontmanager/src/cosmic_system.rs) | cosmic-text (fontdb + rustybuzz + swash) | A second implementation proving the trait is engine-agnostic; see its module doc for why the trait is still somewhat "Parley-shaped". |
| `PangoFontSystem` | [`gosub_renderer_cairo/src/font/pango.rs`](../crates/gosub_renderer_cairo/src/font/pango.rs) | Pango / fontconfig | Registers web fonts into process-global fontconfig. Requires one-time init from the GTK main thread (`init_from_gtk_thread`) to resolve `system-ui`; a process-wide singleton exists for this reason. |
| `SkiaFontSystem` | [`gosub_renderer_skia/src/font/skia.rs`](../crates/gosub_renderer_skia/src/font/skia.rs) | Skia (`skia_safe`) paragraph layout | Measures through the same thread-local `FontCollection` the Skia rasterizer draws with. |

`ParleyFontSystem` and `CosmicFontSystem` live in `gosub_fontmanager` because they are
renderer-independent. `PangoFontSystem` and `SkiaFontSystem` live inside their renderer
crates because they are inherently coupled to that renderer's text engine.

### How a font system reaches layout and rendering

A single instance is shared as `Arc<Mutex<dyn FontSystem>>` between the layouter and the
rasterizer:

- Your config implements `HasFontSystem` (usually via `DefaultRenderConfig<Backend, FontSystem>`,
  see [configuration.md](configuration.md)), which hands the `Arc` to both sides.
- In the render pipeline, `Rasterable::font_system()`
  ([`gosub_render_pipeline/src/rasterizer.rs`](../crates/gosub_render_pipeline/src/rasterizer.rs))
  exposes the rasterizer's font system so the layouter can adopt the same instance. It returns
  `None` for rasterizers that don't shape through a `FontSystem` (e.g. the null rasterizer);
  the layouter then falls back to its own `ParleyFontSystem`
  (`TaffyLayouter::new()` in [`gosub_render_pipeline/src/layouter/taffy.rs`](../crates/gosub_render_pipeline/src/layouter/taffy.rs)).
- `register_font` is how `@font-face` web fonts and bundled fallbacks (Roboto, from
  `gosub_shared`) enter the collection — once, visible to both measurement and drawing.

Measurement happens in **CSS pixels**; DPI scaling is applied later in the pipeline.

## Family 2: text rasterizers (`text_*` features)

Each renderer crate can draw text through more than one text engine. The choice is a cargo
feature, resolved at compile time; each variant lives in `src/rasterizer/text/<engine>.rs`
and exports a `do_paint_text(...)` entry point that paints one `PaintCommand::Text` onto the
backend's surface.

| Renderer crate | Available features | Default |
|---|---|---|
| `gosub_renderer_cairo` | `text_pango`, `text_parley`, `text_skia` | `text_pango` |
| `gosub_renderer_vello` | `text_parley`, `text_skia`, `text_pango` | `text_parley` |
| `gosub_renderer_skia` | — (always uses Skia paragraph layout) | built in |

A `compile_error!` in each crate's `rasterizer/text.rs` guards that at least one `text_*`
feature is enabled; when several are enabled at once, a fixed precedence chain picks which
`do_paint_text` is used.

Layout-building helpers for the rasterizers live under each crate's `src/font/` directory
(e.g. `get_parley_layout` in the Vello crate, `get_skia_paragraph` in the Skia crate) —
distinct from the `FontSystem` implementations that happen to share that directory. Vello
additionally caches shaped text in `backend/text_renderer.rs` (keyed by `TextKey`).

## Which font system pairs with which renderer

Measurement must match drawing, so pick the font system that corresponds to the text
rasterizer your renderer was built with:

| Renderer (text feature) | Font system | Used by |
|---|---|---|
| Cairo (`text_pango`) | `PangoFontSystem` | GTK4/winit/egui cairo examples |
| Vello (`text_parley`) | `ParleyFontSystem` | winit/egui vello examples |
| Skia | `SkiaFontSystem` | skia examples, `bin/gosub-screenshot` |

For example, the screenshot tool uses `DefaultRenderConfig<SkiaBackend, SkiaFontSystem>`.

Mixing (say, `ParleyFontSystem` for measurement with Pango drawing) will compile — the trait
doesn't stop you — but line breaks and box sizes are then computed with different metrics
than the glyphs painted into them, which shows up as clipped or overflowing text.

### Per-implementation quirks worth knowing

- **Pango** uses its own natural line height during measurement, deliberately matching how
  the Cairo rasterizer draws; `TextStyle::line_height` is not applied there.
- **Skia** *does* apply the CSS line height during measurement (as a multiple of font size),
  exactly like its draw path — skipping this used to make measured boxes shorter than the
  painted text. It also prunes the CSS family list so an unavailable leading family can't
  capture the platform default.
- **Pango's `system-ui`** resolution must happen on the GTK main thread before background
  rendering starts.
