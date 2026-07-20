# gosub_fontmanager

Concrete font-system implementations for the Gosub browser engine: font registration,
CSS-query resolution (including raw font bytes), shaping into positioned glyph runs, and
measurement. The `FontSystem` trait itself lives in `gosub_interface`; this crate only
provides implementations. None of them draws glyphs — painting is each render backend's
job.

## The implementations

| Type | Backing libraries | Availability | Typical pairing |
|------|-------------------|--------------|-----------------|
| `ParleyFontSystem` | Parley + Fontique | always | default; used with the Vello backend |
| `CosmicFontSystem` | cosmic-text + rustybuzz + swash | always | pure-Rust alternative, not wired into any default config |
| `PangoFontSystem` | fontconfig + Pango/HarfBuzz | feature `pango` | the Cairo configs |
| `SkiaFontSystem` | Skia textlayout | feature `skia` | the Skia configs |

Each maps the shared `gosub_interface` value types (`TextStyle`, `FontQuery` /
`ResolvedFont` / `FontBlob`, `ShapedText` / `ShapedRun` / `ShapedGlyph`) onto its backing
library. Examples pick a font system in their `type AppConfig` alias via
`DefaultRenderConfig<Backend, FontSystem>`.

## Notes

- No default features; only Parley and Cosmic compile without opting in. `pango` pulls in
  GTK4/pangocairo/fontconfig; `skia` pulls in `skia-safe`.
- The Pango system uses raw fontconfig FFI serialized behind a global mutex (concurrent
  fontconfig configuration mutation can segfault) — this is why the crate relaxes the
  workspace `unsafe_code` lint from forbid to deny.

## Further reading

- [docs/fonts.md](../../docs/fonts.md) — the full picture: font systems vs text
  rasterizers, and the comparison table
- [docs/configuration.md](../../docs/configuration.md) — how a config pairs a backend
  with a font system
