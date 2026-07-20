# gosub_renderer_vello

Vello/wgpu GPU backend for the Gosub render pipeline. Unlike the Cairo/Skia backends,
Vello is a compute-shader scene renderer, not a canvas: the default path rebuilds and
renders the whole viewport as one `Scene` per frame (`raster_strategy() = Sequential`,
`renders_to_gpu_texture() = true`). Setting `GOSUB_VELLO_GPU_TILES=1` opts into the
shared GPU tile compositor instead.

## Entry points

- `VelloBackend<C: WgpuContextProvider>` — the `RenderBackend` implementation, generic
  over the host's wgpu context. Construct with `VelloBackend::new(Arc<C>)`.
- `WgpuContextProvider` — the trait the host implements to share its `wgpu::Device` /
  `Queue` and an id-keyed texture registry with the engine. `gosub_winit` provides a
  ready-made implementation for winit windows; `egui-vello` supplies its own.
- `VelloRasterizer` — the `Rasterable` implementation for the tile path.
- `WgpuResources` — the shared device/queue/renderer bundle.

Text goes through a Parley-based glyph pipeline (`backend/text_renderer.rs`) with its own
font caching, using `ParleyFontSystem` internally.

## Limitations

`snapshot()` is not implemented, so headless snapshotting is not supported on this
backend — use CPU Skia for that (see `gosub-screenshot`).

Used by the `winit-vello` and `egui-vello` examples.

## Further reading

- [docs/render-pipeline/gpu-render-flow.md](../../docs/render-pipeline/gpu-render-flow.md)
  — the GPU presentation flow
- [docs/render-pipeline/backends.md](../../docs/render-pipeline/backends.md) — scene
  rendering vs tile+composite, and when each wins
- [docs/render-pipeline/layering-and-compositing.md](../../docs/render-pipeline/layering-and-compositing.md)
