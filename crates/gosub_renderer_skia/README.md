# gosub_renderer_skia

Skia rasterizer and compositor backend for the Gosub render pipeline. It implements the
`RenderBackend` contract (`name() = "skia"`, raster strategy `ParallelCached`) and
rasterizes tiles on the CPU with `skia-safe`.

A note on "GPU": the crate itself is CPU-raster only — there is no GPU feature flag or
wgpu/GL code here. The `winit-skia-gpu` / `gtk4-skia-gpu` examples get their GPU speedup
entirely host-side, by compositing the CPU-rasterized tiles into the window through
Skia's Ganesh/GL `DirectContext`. Relatedly, `SkiaBackend::render()` is effectively a
headless-snapshot fallback: in normal operation the engine takes the tile-cache path and
never calls it.

## Entry points

- `SkiaBackend` — the `RenderBackend` implementation, with `SkiaSurface` (DPR-aware).
- `SkiaRasterizer` — the per-tile `Rasterable` painter; `::new(dpi_scale_factor)` or
  `::with_font_system(...)`. Text is drawn through Skia's own textlayout.
- `SkiaFontSystem` — re-exported from `gosub_fontmanager` (built with its `skia`
  feature) for measurement/shaping in layout.

Used by the `winit-skia`, `egui-skia`, `gtk4-skia`, `winit-skia-gpu`, and `gtk4-skia-gpu`
examples (`DefaultRenderConfig<SkiaBackend, SkiaFontSystem>`), and by the headless
`gosub-screenshot` tool.

## Further reading

- [docs/render-pipeline/backends.md](../../docs/render-pipeline/backends.md) — backend
  comparison, including the note on `SkiaBackend::render()` and the tile-cache path
- [docs/render-pipeline/gpu-render-flow.md](../../docs/render-pipeline/gpu-render-flow.md)
  — how the GPU examples composite CPU tiles
- [docs/headless.md](../../docs/headless.md) — headless rendering with CPU Skia
