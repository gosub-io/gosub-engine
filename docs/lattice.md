# Lattice: the table layout engine (`gosub_lattice`)

Taffy covers flexbox, grid, and block layout — but not CSS tables. `gosub_lattice` fills
that gap: a standalone implementation of the CSS table layout algorithm that works *in
conjunction with* a general layout engine rather than replacing it. Taffy (or any host)
lays out everything, tables included, as ordinary boxes; lattice then recomputes the table
grid geometry — column widths, row heights, cell positions — and writes it back, while
delegating the layout *inside* each cell right back to the host engine.

## The `TableTree` adapter

Lattice knows nothing about DOM types, style storage, or which layout engine hosts it. All
I/O goes through the `TableTree` trait (`lib.rs`), generic over the host's node-id type:

| Method | Direction | Purpose |
|---|---|---|
| `children`, `table_role`, `css_length`, `attr_usize` | read | tree structure, `display: table-*` roles, widths/heights/borders/padding, `colspan`/`rowspan` |
| `set_layout(id, CellLayout)` | write | computed position/size for groups, rows, and cells |
| `layout_cell(id, available_width) → height` | **callback into the host** | lay out the cell's children with the host engine and report their content height |
| `cell_content_width(id) → width` | read | the cell's natural width from a prior host layout pass |

`layout_cell` and `cell_content_width` are the cooperation points with Taffy: lattice does
grid geometry, the host engine does everything inside the cells.

Two adapters exist — one per [world](two-worlds.md):

- **`PipelineTableTree`** (`gosub_render_pipeline/src/layouter/table.rs`) — the live one.
  Tables are first laid out by Taffy as ordinary blocks (wrong positions, but cell text is
  measured correctly), then `post_process_tables` runs lattice over every `display: table`
  node in two passes — pre-order so column widths flow top-down into nested tables,
  reverse post-order so heights flow bottom-up out of them. See
  [render-pipeline/layout.md](render-pipeline/layout.md#tables-the-lattice-write-back).
- **`gosub_taffy`'s adapter** — the interface-world counterpart (currently dormant along
  with the rest of that crate).

## The algorithm (`compute_table_layout`)

One call per table; returns the table's `(width, height)` border-box size (the caller
positions the table itself in the surrounding flow).

1. **Model building** (`model.rs`) — walk the subtree into a typed `TableModel`: caption,
   column groups, and header/body/footer row groups, classified by `display` role. The CSS
   2.1 §17.2.1 anonymous-box fixups are applied here: a bare row directly under the table
   gets an anonymous body group, a bare cell under a group gets an anonymous row.
   `border-spacing`, `border-collapse`, and `table-layout` are parsed into the model.
2. **Grid placement** (`grid.rs`) — per section, resolve each source cell to a concrete
   `(row, col)` slot with effective `colspan`/`rowspan` (rowspan clamped to its section, so
   nothing spans out of a `<thead>`). The result is a `SectionGrid` that can answer
   "which cells are in row *i*" and "which columns are spanned across a row boundary".
3. **Column widths** (`sizing/columns.rs`) — available space is the table width minus all
   border-spacing gutters. The first non-empty row is scanned: single-column cells with an
   explicit CSS width get it (clamped to at least their content's natural width — a
   `width: 18px` cell holding a 20 px image must not clip it). Remaining space goes to the
   auto columns **proportionally to their natural content width** (from
   `cell_content_width`), with a threshold heuristic: narrow columns (< 50 px intrinsic —
   rank numbers, vote buttons) keep their natural width with a 14 px floor, wide content
   columns share what's left. Equal distribution is the fallback when no content-width
   data exists (mock trees).
4. **Row heights** (`sizing/rows.rs`) — per non-spanning cell: `layout_cell(inner_width)`
   asks the host to lay out the cell's children at the now-final column width; the row
   height is the max over its cells of `max(content height, explicit CSS height) + border
   + padding`. Explicit height is a *minimum* — content can grow past it.
5. **Placement** (`compute.rs`) — sections render header → body → footer regardless of
   source order, per CSS. Groups are positioned relative to the table, rows relative to
   their group, cells relative to their row; spanning cells sum the widths/heights of the
   columns/rows they cover. Everything is written back through `set_layout` as *relative*
   positions — the adapter converts to absolute coordinates (the pipeline's does so in
   `apply_positions`).

## Trying it standalone

The crate is self-contained enough to play with in isolation:

- `mock.rs` provides a `MockTable` builder implementing `TableTree` with no DOM at all —
  it powers the unit tests.
- `cargo run --bin table_console -p gosub_lattice` renders demo tables (spans, explicit
  widths, headers/footers) as ASCII grids in the terminal — handy for eyeballing algorithm
  changes without a browser build.

## Current limitations

- **`rowspan > 1` heights**: spanning cells are skipped during row-height computation;
  distributing their height across the spanned rows is deferred.
- **`border-collapse`, `table-layout: fixed`, captions**: parsed into the model but not
  yet consumed by the algorithm — layout always uses the separate-borders model with
  auto sizing, and captions get no box.
- Column-width resolution scans only the first non-empty row for explicit widths, rather
  than the full min/max-content pass of the spec's auto algorithm.
