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
Serve the directory and point the engine at it, e.g.:

```sh
cargo run --example screenshot_url -p gosub_render_pipeline --features backend_cairo -- \
  http://localhost:8000/image-test.html 1000 1400
```

(`python3 -m http.server` from this directory works as a quick server.)
