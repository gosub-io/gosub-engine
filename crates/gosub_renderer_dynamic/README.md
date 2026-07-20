# gosub_renderer_dynamic

Runtime-selectable render backend: `DynamicRenderBackend` bundles the Cairo, Skia, and
Vello backends behind a single `RenderBackend` and delegates every call to the active
one. This is the only place in the workspace that knows the concrete backends exist — the
pipeline and engine only ever see `dyn RenderBackend`.

Feature flags decide which backends are *compiled in* (`cairo`, `skia`, `vello`, each
opt-in so a host builds only what its platform can construct); which one is *active* is a
runtime choice via the builder or `set_active`, stored lock-free in an atomic so it can
be switched while running. An always-available `NullBackend` is the fallback when the
selected kind isn't registered.

## Usage

```rust
let backend = DynamicRenderBackend::builder()
    .with_cairo()                       // feature "cairo"
    .with_vello(wgpu_context.clone())?  // feature "vello"; fallible (needs a wgpu device)
    .active(RenderBackendKind::Cairo)
    .build();

backend.set_active(RenderBackendKind::Vello);
```

None of the bundled examples uses this crate yet — they each wire a single concrete
backend via `DefaultRenderConfig`. A delegation test (`forwards_every_defaulted_method`)
guards against forgetting to forward a defaulted trait method, which silently broke
Vello's GPU path once already.

## Further reading

- [docs/render-pipeline/backends.md](../../docs/render-pipeline/backends.md) — the
  concrete backends this crate bundles
- [docs/configuration.md](../../docs/configuration.md) — compile-time backend selection,
  which this crate complements at runtime
