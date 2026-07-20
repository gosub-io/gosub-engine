# gosub_lattice

The CSS table layout engine of the Gosub workspace — the table algorithm (CSS 2.1 §17)
that general-purpose flex/grid layouters don't cover. The crate is deliberately
standalone: its only dependency is `anyhow`, and it talks to the host layout engine
exclusively through the `TableTree` adapter trait.

## How it plugs in

The host implements `TableTree` (node traversal, `table_role`, CSS property access, a
`layout_cell` callback for measuring cell content, and `set_layout` write-back), then
calls `compute_table_layout` once per table. The live adapter is `PipelineTableTree` in
`gosub_render_pipeline`; `mock::MockTable` provides a DOM-free implementation for tests
and experiments.

## What lives here

| Module | Role |
|--------|------|
| `model` | Walks the subtree into a typed `TableModel`, with CSS 2.1 anonymous-box fixups |
| `grid` | Grid placement: resolves cells to `(row, col)` honoring colspan/rowspan |
| `sizing` | Column-width algorithm and row heights (via the `layout_cell` callback) |
| `compute` | `compute_table_layout`: section ordering, placement, write-back |
| `types` | The flat data types crossing the adapter boundary (`CellLayout`, `CssLength`, ...) |
| `mock` | `MockTable` builder for standalone use |

## Trying it

`cargo run --bin table_console -p gosub_lattice` renders demo tables as ASCII grids.

## Known limitations

`rowspan > 1` heights are not distributed yet; `border-collapse`, `table-layout: fixed`,
and captions are parsed into the model but not consumed (layout always uses
separate-borders + auto sizing); column widths scan only the first non-empty row.

## Further reading

- [docs/lattice.md](../../docs/lattice.md) — the algorithm and the `TableTree` contract
- [docs/render-pipeline/layout.md](../../docs/render-pipeline/layout.md) — the lattice
  write-back into the pipeline's layouter
- [docs/two-worlds.md](../../docs/two-worlds.md) — why lattice cooperates with the
  general layouter instead of replacing it
