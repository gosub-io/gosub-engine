# Headless usage

How to drive the full engine — navigation, layout, rasterization — without opening a
window. The reference implementation is the screenshot tool
([`bin/gosub-screenshot/main.rs`](../bin/gosub-screenshot/main.rs)): URL in, full-page PNG
out.

```sh
cargo run --release -p gosub-screenshot -- news.ycombinator.com out.png 1280
```

Two levels of "headless" exist, pick per use case:

- **Engine without rendering** — the single-file examples (`hello-world`, `multi-tab`, …)
  run on the `NullBackend`: navigation, parsing, and events work, nothing is rasterized.
  Good for crawling and parser work. See [examples.md](examples.md).
- **Engine with real rendering, no window** — this page: the screenshot tool renders
  pages exactly as a GUI browser would, using CPU Skia.

## Why CPU Skia

The tool uses `DefaultRenderConfig<SkiaBackend, SkiaFontSystem>`:

- **No GPU, no window system**: no wgpu adapter is requested, and skia-safe is statically
  linked, so the binary runs on a bare server/CI container with no system libraries.
- **No size limit**: the page is rasterized into small cached tiles
  (`ExternalHandle::TileCache`) that the tool composites itself, so — unlike a
  GPU-texture path — there is no texture-size cap and a page of *any* height can be
  captured in one image.

The font system must match the backend (measurement and drawing share one font
collection) — see [fonts.md](fonts.md).

## Engine setup

The setup is the same as a GUI embedder's, minus the window (compare
[tutorial.md](tutorial.md)):

1. Create the backend and a `DefaultCompositor` whose redraw callback signals a channel —
   this is the "new frame available" notification a GUI would use to schedule a repaint.
2. `GosubEngine::<AppConfig>::new(…)`, `start()`, `subscribe_events()`.
3. Create a zone with throwaway storage (`SqliteLocalStore::new(":memory:")`, in-memory
   session store, no cookie persistence) and one tab.
4. Send `TabCommand::SetViewport`, `Navigate`, and `ResumeDrawing { fps }`.

One trick worth stealing: the tool sets the initial viewport to `width × 16384` CSS px.
The tall viewport makes below-the-fold and lazily-sized content lay out immediately; the
final image uses the page's *true* laid-out height, not this value.

## Capturing a frame

The tool runs three phases:

1. **Wait for content** — spin on the event stream until `NavigationEvent::Finished`, then
   wait for the first redraw signal after that (the first frame that actually contains the
   page). Both waits have timeouts (`--nav-timeout`, `--render-timeout`).
2. **Obtain the tile cache** — ask the compositor for the tab's latest frame
   (`compositor.frame_for(tab_id)`) and expect an `ExternalHandle::TileCache`, which
   carries the tiles and the laid-out `page_height`. A 1-px synthetic scroll
   (`TabCommand::MouseScroll`) nudges the engine into publishing a fresh tile-cache frame
   if needed.
3. **Composite** — allocate an opaque-white RGBA buffer at `viewport_width × page_height`
   and blend every tile into it. This is a miniature version of what every host compositor
   does (see [layering-and-compositing.md](render-pipeline/layering-and-compositing.md)):
   - normalize the tile's pixel format to RGBA via `PixelFormat::to_rgba` (tiles are
     self-describing; Skia produces premultiplied `[B,G,R,A]`);
   - scale by the tile's layer **group opacity** (a translucent navbar fades as a unit);
   - **source-over blend** into the buffer rather than overwrite — a promoted
     `position: fixed` layer's transparent tile regions must reveal the rows already
     composited beneath, not erase them.

Scroll anchors are irrelevant here because the capture is the full page at scroll 0.

## CLI reference

```text
gosub-screenshot <url> [output.png] [width]
    --nav-timeout <s>      wait for navigation (default 30)
    --render-timeout <s>   wait for first render after navigation (default 120)
```

`https://` is prepended when the URL has no scheme. The build embeds the git SHA and date
via `build.rs` (`gosub-screenshot --version`).
