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
| `05-caption.html` | `<caption>`, `caption-side: bottom` | caption layout (**gap**) |
| `06-colspan.html` | colspan 2/3/full/overflowing | slot filling, span clamping |
| `07-rowspan.html` | rowspan, tall spanning content, span past last row | rowspan height distribution (**gap**) |
| `08-span-mix.html` | interlocking colspan+rowspan block puzzle | slot filling |
| `09-widths-explicit.html` | px/% cell widths; explicit width on a row-2 cell | column algorithm beyond first row (**gap**) |
| `10-table-width.html` | table width auto/px/%/100% | shrink-to-fit for auto (**gap**) |
| `11-fixed-layout.html` | `table-layout: fixed` semantics | fixed layout (**gap**) |
| `12-colgroup.html` | widths from `<col>`/`<colgroup>` | col widths (**gap**) |
| `13-content-extremes.html` | unbreakable word, over-wide block | min-content column floors (**gap**) |
| `14-border-spacing.html` | spacing 0 / 10px / asymmetric; padding | separate border model |
| `15-border-collapse.html` | separate vs collapse, border conflict | border-collapse (**gap**) |
| `16-nested-layout.html` | flex + block stacks + wrapping text in cells | `layout_cell` callback |
| `17-nested-table.html` | table inside a table cell | nested `compute_table_layout` |
| `18-vertical-align.html` | top/middle/bottom/baseline | vertical-align (**gap**) |
| `19-infobox.html` | wikipedia-style infobox (fixed width, spans, label col) | several combined |
| `20-data-table.html` | full-width data table, zebra, collapse, tfoot | several combined |

Pages marked **gap** exercise features `compute.rs` does not implement yet
(they parse into the model but are ignored); they document target behavior
and double as acceptance tests for the corresponding roadmap item.

## Findings from the first render pass (2026-08-06, Cairo backend)

Solid: slot filling (02, 06, 08 place every span correctly, clamped spans
included), section reordering (03), anonymous boxes (04), first-row px/%
widths (09, table 1), table width px/%/100% (10), separate borders with
per-cell overrides (15), and both real-world pages (19, 20) are close.

Broken or missing, beyond the known compute gaps:

- **Auto columns sized from row 1 only** - in 01, "seven"/"eight" clip
  because rows 2+ never influence the natural width; 08 degenerates badly.
- ~~**Block children in cells lay out horizontally**~~ - **fixed
  2026-08-07**: cells now use block inner layout in the first pass
  (`css_taffy_converter.rs`), and `layout_cell` re-runs taffy on the cell
  subtree at the final lattice width (`TaffyLayouter::relayout_cell`), so
  stacked blocks, wrapping, and `text-align` all resolve against real cell
  geometry. Cells hosting nested tables keep the first-pass approximation
  (re-layout would clobber the inner table's lattice output).
- **No min-content floor** - 13: an unbreakable word squeezes neighbors
  into clipping; a 350px block overflows its column instead of widening it.
- **`border-spacing` CSS never reaches the algorithm** - 14 renders
  identical gutters for 0 / 10px / 20px 4px (the mock-tree tests pass
  because they inject spacing directly).
- **Nested tables collapse to a sliver** (17).
- **`text-align` on cells ignored** (20, numeric columns).
- Confirmed compute gaps as expected: captions absent (05), rowspan height
  not distributed (07), later-row explicit widths ignored (09 table 2),
  no shrink-to-fit auto width (10), `<col>` widths ignored (12),
  border-collapse (15), vertical-align (18).
