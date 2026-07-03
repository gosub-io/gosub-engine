# Layout (Stage 2 in depth)

How the pipeline's layouter turns a `RenderTree` into a `LayoutTree` of absolutely-positioned box models. This expands on [Stage 2 in stages.md](stages.md#stage-2--layout).

The layouter lives in `crates/gosub_render_pipeline/src/layouter/` and is built around the [Taffy](https://github.com/DioxusLabs/taffy) layout engine — but Taffy is an implementation detail. The `CanLayout` trait (`layouter.rs`) takes a `RenderTree` and returns a `LayoutTree`; everything downstream (layering, painting, tiling) sees only `BoxModel`s, so a different layout engine could be swapped in.

## The pass structure

`TaffyLayouter::layout` runs four steps:

1. **Tree generation** (`generate_taffy_element`) — one recursive walk of the render tree builds *two* trees in parallel: the internal `TaffyTree` (styles + measure contexts) and the pipeline's `LayoutTree` arena (`LayoutElementNode`s). A mapping table links each layout element to its Taffy node. Inline children get wrapped in anonymous flex containers along the way (see below).
2. **Taffy compute** (`compute_layout_with_measure`) — Taffy solves the flex/grid/block constraints, calling back into a measure function for leaf content (text, images, SVG). The viewport gives the available space; without one, layout runs at max-content.
3. **Box-model population** (`populate_boxmodel`) — Taffy positions are parent-relative; this recursive pass accumulates offsets into absolute page coordinates and converts every node to a `BoxModel` (margin / border / padding / content rects). After this, Taffy state is no longer consulted.
4. **Table post-processing** (`post_process_tables`) — `display: table` subtrees are re-laid-out by `gosub_lattice` and the corrected positions are written back over the Taffy results (see below).

Two global settings matter here: Taffy's **rounding is disabled** (its integer-pixel snapping truncated fractional text widths, e.g. 52.344 → 52.0, making Pango wrap text that Parley measured as fitting), and all measurement happens in **CSS pixels** — DPI scaling is applied later in the pipeline.

## CSS → Taffy styles

`CssTaffyConverter` (`css_taffy_converter.rs`) maps a node's computed style onto Taffy's `Style`: display (block/flex/grid/none), position + insets, size/min/max, margin/padding/border widths, flex direction/wrap/basis/grow/shrink, alignment (`align-*`/`justify-*`), gap, overflow, `box-sizing`, and text-align. Grid support includes parsing `grid-template-columns/rows` (with `repeat()`, `fr`, `minmax()`), `grid-auto-flow`, and line-based placement (`grid-row`/`grid-column`, including spans). Font-relative units (`em`, `ch`) on non-font properties are resolved against the element's computed font-size.

## Inline content: the anonymous-flex emulation

Taffy has no inline formatting context, so the layouter emulates one. When a block element has inline children (inline elements, inline-blocks, text nodes), they are collected into runs and each run is wrapped in an **anonymous flex container** (`display: flex; flex-wrap: wrap; align-content: flex-start`) inserted between the parent and the children in the Taffy tree only — the `LayoutTree` never sees it.

Details that make this work:

- **Every** inline run gets a container, even a single text node — the flex algorithm then hands the measure function a definite available width, so text wraps instead of laying out at max-content and overflowing.
- **`<br>`** is recorded as a break marker, not a flex item. A run containing breaks is split into one anonymous container *per line box*, which the block parent stacks vertically. A standalone `<br>` emits an empty container pinned to the break's line-height, so consecutive `<br>`s produce blank lines instead of collapsing.
- **Whitespace-only text nodes** between elements (e.g. between `</span><span>`) are kept as a single non-breaking space with an explicit ~0.3 em width and `flex-shrink: 0` — Parley measures a lone space as zero-width at min-content, which would collapse the gap. Leading and trailing whitespace runs are dropped.
- **Flex/grid parents skip the wrapping entirely**: in those formatting contexts every child is a direct layout participant, and an extra container would break `gap` and alignment.
- Since the anonymous container exists only in the Taffy tree, `populate_boxmodel` consults `anon_container_map` to add the container's own offset when computing an inline child's absolute position. Inline elements also don't establish a containing block: their children inherit the *enclosing block's* content width as their wrap limit (`ElementContextText::available_width`), so the renderer wraps at the same boundary the measure pass used.

This flex emulation is an approximation (each inline element is a rigid flex item, so a long inline span wraps as a unit rather than flowing across lines). A proper styled-inline-run implementation is staged in `layouter/inline_run.rs` — currently unwired scaffolding; its module doc describes the staged rework plan.

## Text measurement

Leaf nodes carry a `TaffyContext` so the measure callback knows what it is sizing. For text (`TaffyContext::Text`):

- The text is prepared at tree-generation time: `white-space: normal` collapsing (source indentation would otherwise render as blank lines), preservation of one leading/trailing inter-element gap as a non-breaking space, and `text-transform` — applied *before* measurement so the measured width and the painted glyphs always agree.
- Font parameters (family, size, weight, style, line-height, decoration) come from computed CSS. `line-height: normal` resolves to **1.4 × font-size** — deliberately above the spec's ~1.2, because Parley (measurement) and Pango (Cairo's rasterizer) read different font metrics tables, and the buffer keeps descenders inside the box that layout reserved.
- Measurement goes through the shared `FontSystem` (`layouter/text/parley.rs` → `FontSystem::measure`), the same instance the rasterizer draws with — see [fonts.md](../fonts.md). The mutex is locked per call, not for the whole pass.
- Results are **memoized** in `measure_cache`, keyed by (text, family, size, line-height, weight, max-width): Taffy probes each node 2–4× (min-content, max-content, final width), and caching removes the redundant shaping calls.
- `white-space: nowrap` measures at effectively unlimited width and sets `flex-shrink: 0`.
- Widths and heights are **ceiled** to whole CSS pixels: Taffy feeds the f32-truncated width back as the available width on the next probe, and without the ceiling the text re-measures into slightly less space than it needs and wraps spuriously.

## Replaced elements (images, SVG)

`<img>` elements resolve their `src` against the document base URL and request it from the shared `MediaStore` — **non-blocking**: an uncached image starts a background fetch and layout continues with a placeholder size (the HTML `width`/`height` attributes if present, else whatever CSS produced); the completed fetch triggers a reflow that installs the real intrinsic size. A failed load measures as a fixed 32×32 so the broken-image icon can't blow up the layout. Inline `<svg>` elements are serialized back to markup and loaded into the media store the same way.

At measure time, `measure_replaced` honours whichever dimension CSS constrained and derives the other from the intrinsic aspect ratio — a `height: 30px` logo keeps its shape instead of stretching to its intrinsic width.

Layout also resolves each element's CSS `background-image` into the media store (`LayoutElementNode::background_media`), recording whether it is raster or SVG so the painter can pick the right paint path. The media store must be shared with the rasterizer (`set_media_store`) — otherwise the resources loaded here aren't visible when tiles are painted.

## From Taffy layout to `BoxModel`

Taffy reports a node's border-box size, location (relative to its parent), and padding/border/margin edge widths. `populate_boxmodel` walks the layout tree accumulating absolute offsets and calls `BoxModel::new`, which derives the four rects from the border box: the margin box grows outward by the margins, the padding box shrinks inward by the borders, and the content box shrinks further by the padding. The final `LayoutTree` therefore carries absolute page-space rects only, plus each element's `parent` link (used later, e.g. to find the sticky cage in [layering](layering-and-compositing.md)). The root's margin box becomes `root_dimension` — the full page size the tiler subdivides.

## Tables: the lattice write-back

Taffy doesn't implement CSS table layout, so tables get a second pass through the `gosub_lattice` crate (`layouter/table.rs`):

1. During the Taffy pass, table elements are laid out as ordinary blocks — wrong positions, but text inside cells is measured correctly.
2. `post_process_tables` collects all `display: table` nodes in pre-order and runs lattice on each via `PipelineTableTree`, an adapter implementing lattice's `TableTree` trait over the pipeline document + layout tree. The adapter answers lattice's questions from data the Taffy pass already produced: cell heights reuse the Taffy-measured content height, and intrinsic column widths come from the *measured* text/image widths of the leaf nodes (not the equal-distributed Taffy cell widths), which keeps narrow columns narrow.
3. It runs **twice**: a pre-order pass (outer tables before nested ones) so column widths flow top-down — a nested table reads its available width from its already-sized parent cell — and a reverse, post-order pass so heights flow bottom-up — an outer cell grows to contain its nested table's true height.
4. `apply_positions` converts lattice's relative cell layouts to absolute `BoxModel`s and *translates* every non-table descendant of a moved cell by the cell's displacement, so the content Taffy laid out inside the cell moves along with it.

## Known limitations

- Inline layout is the flex approximation described above (rigid inline items, no cross-line flow); the inline-run rework addresses this.
- Table cell heights reuse measurements made in a flex context — an approximation that covers the common single-column-of-text case.
- `float` is not implemented; `text-transform: full-width` and other exotic keywords pass through unchanged.
- Media-dependent layout is eventually-consistent: pages with uncached images lay out with placeholder sizes first and reflow when fetches complete.
