# gosub_renderer_cairo

Cairo rasterizer and compositor backend for the Gosub render pipeline — a CPU backend
built on gtk4/cairo, with Pango text. It implements the `RenderBackend` contract
(`name() = "cairo"`, raster strategy `ParallelCached`: tiles are rasterized in parallel
and cached, then composited by the host).

## Entry points

- `CairoBackend` — the `RenderBackend` implementation, with `CairoSurface`.
- `CairoRasterizer` — the per-tile `Rasterable` painter; `::with_font_system(...)` to
  share a `FontSystem`, or `::new()` to let the layouter fall back to its own.
- `init_gtk_resources()` (feature `pango`) — must run on the main thread before
  background rendering when used outside a GTK window (egui/winit/headless); for headless
  use set `GDK_BACKEND=offscreen`.

## Structure and features

`backend.rs` holds the DPR-aware surface and tile blitting (via cairo's
`create_for_data_unsafe`, the reason this crate relaxes the workspace `unsafe_code` lint
to deny); `rasterizer/` has one module per command family (`brush`, `rectangle`, `svg`,
`text`), with SVG going through resvg.

The default `pango` feature pulls in GTK4 and `gosub_fontmanager`'s `PangoFontSystem` for
measuring/shaping, plus the gdk-pixbuf image-brush path.

Used by the `winit-cairo`, `egui-cairo`, and `gtk4-cairo` examples
(`DefaultRenderConfig<CairoBackend, PangoFontSystem>`).

## Further reading

- [docs/render-pipeline/backends.md](../../docs/render-pipeline/backends.md) — how the
  three backends differ (tile+composite vs scene)
- [docs/fonts.md](../../docs/fonts.md) — the Pango font system
- [docs/headless.md](../../docs/headless.md) — the GDK offscreen note
