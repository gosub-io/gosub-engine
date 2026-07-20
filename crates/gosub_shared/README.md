# gosub_shared

Foundation types shared across the Gosub workspace. This crate sits under nearly every
other crate and holds the vocabulary they exchange: the workspace `Result`/`Error`
aliases, geometry, opaque ids, and the encoding-aware byte stream the parsers consume.

## What lives here

| Module | Contents |
|--------|----------|
| `types` | The workspace `Result<T>` alias, `Error`, `ParseError`, generic `Size`/`Point`/`Rect` |
| `geo` | Concrete `f32` geometry aliases (`Point`, `Size`, `Rect`, `FP`) |
| `byte_stream` | `ByteStream` / `Encoding` / `Location` — the input stream for the HTML and CSS parsers |
| `node`, `tab_id` | Opaque `NodeId` and `TabId` handles |
| `font` | `Glyph` / `GlyphID` positioned-glyph types |
| `animation` | `Easing` — CSS-like timing functions, backend-agnostic |
| `async_executor` | `spawn` that maps to native threads or wasm `spawn_local` |
| `timing` | Timer registry (`Instant` native, `performance` on wasm) |
| `errors` | `CssError` with source location |

Also embeds `ROBOTO_FONT` (Roboto Regular) as the guaranteed-available fallback font.

Platform differences are handled with `cfg(target_arch = "wasm32")` gates rather than
Cargo features; the crate has none.

## Further reading

- [docs/bytestream.md](docs/bytestream.md) — the byte stream design (crate-local doc)
- [docs/crates.md](../../docs/crates.md) — where this crate sits in the workspace
