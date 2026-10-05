# Image decoding & rendering test

`image-test.html` exercises image decoding and rendering across formats and the
common ways images appear on a page.

## Formats covered
PNG (RGBA/transparency), JPEG (lossy), GIF (palette + transparent key), SVG
(vector), WebP (lossy). All images are static (non-animated).

## What the page tests
1. **Inline `<img>`** — one per format.
2. **CSS `background-image`** — one per format, with `cover` / `contain`.
3. **`background-repeat`** — tiling a small PNG and a small SVG.
4. **`background-size`** — `cover` vs `contain` on a wide (800×200) image.
5. **Scaling** — the same SVG and PNG at several widths.
6. **Edge cases** — a missing image (broken/placeholder) and a 1×1 data-URI PNG
   scaled up.

The page background is a CSS checkerboard so transparent regions in the PNG
(punched circle) and GIF (corner) reveal the pattern underneath — an alpha
compositing check.

## Assets
Binary assets live in `assets/`. Regenerate the raster ones with:

```sh
python3 generate_assets.py   # needs Pillow with WebP support
```

`assets/vector.svg` and `assets/tile.svg` are hand-written text and are not
produced by the script.

## Rendering it
Serve this directory and point `gosub-screenshot` at it. `python3 -m http.server` from here
works as a quick server; the arguments are `<url> [output.png] [width]`.

```sh
cargo run -p gosub-screenshot -- http://localhost:8000/image-test.html out.png 1000
```

That is the default CPU Skia backend. To render the same page through Cairo/Pango instead —
useful for comparing the two rasterizers against each other:

```sh
cargo run -p gosub-screenshot --no-default-features --features backend_cairo -- \
  http://localhost:8000/image-test.html out-cairo.png 1000
```

See [`docs/headless.md`](../../../docs/headless.md) for how the tool drives the engine, and
[`docs/render-pipeline/backends.md`](../../../docs/render-pipeline/backends.md) for what
differs between the backends.
