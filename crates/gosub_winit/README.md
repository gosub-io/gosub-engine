# gosub_winit

winit + wgpu window presentation glue for the Gosub Vello backend. Every winit-based GPU
embedder needs the same two pieces of boilerplate; this crate provides both so examples
and user agents don't repeat them:

- `WinitWgpuContextProvider` — implements `gosub_renderer_vello::WgpuContextProvider`;
  owns the shared `wgpu::Device` / `Queue` and an id-keyed registry of engine-created
  textures.
- `GpuPresenter` — a wgpu surface plus full-screen blit pipeline. Adapter selection passes
  `compatible_surface` (avoiding the Wayland/X11 "adapter can't render to surface" trap)
  and picks a non-sRGB swap-chain format so already-encoded bytes aren't double-encoded.
  `present(...)` shows a Vello-rendered texture; `present_rgba(...)` is the CPU
  tile-cache fallback path.

The consumer to read is the [`winit-vello` example](../../examples/winit-vello) — it
builds a `GpuPresenter` for the window, a `WinitWgpuContextProvider` on the same GPU, and
hands the provider to `VelloBackend`. The other winit examples (`winit-cairo`,
`winit-skia`, `winit-skia-gpu`) use different presentation paths and don't need this
crate.

## Further reading

- [docs/examples.md](../../docs/examples.md) — the GUI example matrix
- [docs/render-pipeline/](../../docs/render-pipeline/) — the pipeline the Vello backend
  renders for
