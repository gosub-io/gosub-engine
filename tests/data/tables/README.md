# Table layout fixture pages

Small, single-purpose HTML pages for exercising `gosub_lattice` (the CSS
table layout engine) through the full render pipeline. Each page states in
its intro paragraph what the correct rendering looks like, so a screenshot
can be judged on its own.

Render one headlessly (no window) with:

```
cargo run -p gosub-screenshot -- file://$PWD/tests/data/tables/01-basic-grid.html out.png
```

## Pages

| Page | Exercises | Depends on |
|------|-----------|------------|
| `01-basic-grid.html` | 3x3 grid, auto columns, border-spacing | core grid + auto widths |
| `02-ragged-rows.html` | rows of unequal length, empty cells | slot filling |
| `03-sections.html` | tfoot before thead in source, multiple tbodies | header->body->footer ordering |
| `04-anonymous-boxes.html` | `display: table` divs, cells without a row | anonymous box generation |
| `05-caption.html` | `<caption>`, `caption-side: bottom` | caption layout (done 2026-08-08) |
| `06-colspan.html` | colspan 2/3/full/overflowing | slot filling, span clamping |
| `07-rowspan.html` | rowspan, tall spanning content, span past last row | rowspan height distribution (done 2026-08-08) |
| `08-span-mix.html` | interlocking colspan+rowspan block puzzle | slot filling |
| `09-widths-explicit.html` | px/% cell widths; explicit width on a row-2 cell | column algorithm beyond first row (**gap**) |
| `10-table-width.html` | table width auto/px/%/100% | shrink-to-fit for auto (**gap**) |
| `11-fixed-layout.html` | `table-layout: fixed` semantics | fixed layout (done 2026-08-07) |
| `12-colgroup.html` | widths from `<col>`/`<colgroup>` | col widths (done 2026-08-07) |
| `13-content-extremes.html` | unbreakable word, over-wide block | min-content column floors (**gap**) |
| `14-border-spacing.html` | spacing 0 / 10px / asymmetric; padding | separate border model |
| `15-border-collapse.html` | separate vs collapse, border conflict | border-collapse + conflict resolution (done 2026-08-09) |
| `16-nested-layout.html` | flex + block stacks + wrapping text in cells | `layout_cell` callback |
| `17-nested-table.html` | table inside a table cell | nested `compute_table_layout` |
| `18-vertical-align.html` | top/middle/bottom/baseline | vertical-align (done 2026-08-08; baseline ~ top) |
| `19-infobox.html` | wikipedia-style infobox (fixed width, spans, label col) | several combined |
| `20-data-table.html` | full-width data table, zebra, collapse, tfoot | several combined |
| `21-section-spans.html` | rowspan overrun + rowspan=0 vs thead/tbody/tfoot | section clamping, HTML rowspan=0 (done 2026-08-12) |

Pages marked **gap** exercise features `compute.rs` does not implement yet
(they parse into the model but are ignored); they document target behavior
and double as acceptance tests for the corresponding roadmap item.

## Findings from the first render pass (2026-08-06, Cairo backend)

Solid: slot filling (02, 06, 08 place every span correctly, clamped spans
included), section reordering (03), anonymous boxes (04), first-row px/%
widths (09, table 1), table width px/%/100% (10), separate borders with
per-cell overrides (15), and both real-world pages (19, 20) are close.

Broken or missing, beyond the known compute gaps:

- ~~**Auto columns sized from row 1 only**~~ - fixed 2026-08-08 by the real
  min/max-content algorithm (see below).
- ~~**Block children in cells lay out horizontally**~~ - **fixed
  2026-08-07**: cells now use block inner layout in the first pass
  (`css_taffy_converter.rs`), and `layout_cell` re-runs taffy on the cell
  subtree at the final lattice width (`TaffyLayouter::relayout_cell`), so
  stacked blocks, wrapping, and `text-align` all resolve against real cell
  geometry. Cells hosting nested tables keep the first-pass approximation
  (re-layout would clobber the inner table's lattice output).
- ~~**No min-content floor**~~ - fixed 2026-08-08 (min/max-content
  algorithm, see below).
- ~~**`border-spacing` CSS never reaches the algorithm**~~ - **fixed
  2026-08-07**: `StyleProperty::BorderSpacingX/Y` added (two internal
  longhands over the one `border-spacing` declaration; X = first length,
  Y = second), wired through the cascade (inherited, UA default
  `table { border-spacing: 2px }`), inline styles, and the
  `PipelineTableTree` adapter. 14 renders 0 / 10px / 20px 4px correctly.
- ~~**Nested tables collapse to a sliver**~~ (17) - fixed 2026-08-08 as a
  side effect of intrinsic column sizing.
- ~~**`text-align` on cells ignored**~~ (20) - fixed 2026-08-07 by the
  cell-content block layout + per-cell re-layout change.
- **Fixed 2026-08-08 (captions + border-collapse)** - the last two compute
  gaps. Captions (05): measured like a full-width cell, placed above or below
  the grid per `caption-side`. Border-collapse (15, 20): spacing forced to 0;
  cells sit flush.
- **Fixed 2026-08-11 (Chrome side-by-side review)**: (1) trailing columns no
  cell originates in are truncated, so an overflowing colspan no longer
  manufactures phantom gutter-bearing columns (06); (2) when lattice's table
  height differs from the first-pass estimate, the document flow below shifts
  and ancestors grow - restoring `<br>` gaps between tables (04, 14), fixing
  sibling overlap and the clipped page bottom (05, 19); (3) collapse-suppressed
  border edges take no layout space, so every collapsed boundary is exactly
  one border wide like a browser (15, 20); (4) `text-align: -webkit-center`
  (the UA caption rule) now centers captions (05).
- **Fixed 2026-08-09 (border-conflict resolution)**: every shared boundary is
  painted by exactly one cell - lattice resolves the winner (wider border
  wins; ties go to the left/top cell, per CSS 2 §17.6.2.1 for same-style
  borders) and flags the loser's edge in `CellLayout.suppressed_borders`;
  the painter skips suppressed edges. The 6px red conflict in 15 now wins all
  four edges. Not implemented: the border-style rank tiebreak (double >
  solid > ...) and row/rowgroup/table borders as conflict participants.
  Every fixture gap is now closed.
- **Fixed 2026-08-08 (min/max-content auto columns)**: the heuristic column
  algorithm was replaced with the CSS 2 §17.5.2.2 algorithm. Every cell
  contributes real min/max-content widths (`TableTree::cell_intrinsic_widths`,
  measured by taffy at MinContent/MaxContent), specified widths are read from
  **all** rows (09), colspan cells distribute their requirement over spanned
  columns weighted by content, auto tables shrink-to-fit (01, 08, 10),
  min-content floors prevent clipping (13), and nested tables (17) now render
  correctly because the host column sizes to the inner table's needs. 19/20
  are essentially browser-accurate.
- **Fixed 2026-08-08**: rowspan height distribution (07) - spanning cells are
  measured and their deficit spreads equally over the spanned rows - and
  vertical-align (18) - lattice emits a `content_offset_y` per cell, the
  pipeline shifts re-anchored cell subtrees by it; alignment resolves by
  walking cell->row->section (HTML-spec `inherit`-on-cells pattern), so the
  browser's `middle` default falls out of the UA stylesheet. `baseline` is
  approximated as `top` until real first-line metrics exist.
- **Fixed 2026-08-07**: `table-layout: fixed` (11) and `<col>`/`<colgroup>`
  widths (12) - new `TableSizing::Fixed` path in `sizing/columns.rs`
  (col elements -> first-row cells with colspan division -> equal split of the
  remainder; content never measured), col widths also seed auto layout;
  pipeline grew `Display::TableColumn(Group)` (no box generated) and the
  `table-layout` style property (Px(1.0) sentinel to lattice).
