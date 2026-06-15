# gosub_lattice

`gosub_lattice` is the CSS **table layout** engine for the Gosub browser engine. It takes a
table's structure and CSS properties and computes the position and border-box size of every
table-internal box — the table itself, row groups (`thead`/`tbody`/`tfoot`), rows, and cells.

It is deliberately **freestanding**: its only dependencies are `anyhow` and `log`. It does not
depend on the rest of the Gosub engine, on any DOM type, or on any particular layout engine.
Instead it talks to the host through a single trait, [`TableTree`](#the-tabletree-trait), so it
can be driven by Taffy, by a mock tree in tests, or by anything else that can answer questions
about a tree of nodes.

> **Why a separate table engine?** CSS table layout (CSS 2.1 §17) does not fit the box model
> that general-purpose layout engines like Taffy implement: column widths depend on the content
> of *every* row, `rowspan`/`colspan` create a 2D slot grid, and anonymous boxes must be
> generated around stray rows and cells. Lattice handles that table-specific logic and hands the
> results back to the surrounding layout engine.

## Contents

- [Mental model](#mental-model)
- [The `TableTree` trait](#the-tabletree-trait)
- [The layout pipeline](#the-layout-pipeline)
- [Public types](#public-types)
- [Coordinate system](#coordinate-system)
- [Integrating lattice](#integrating-lattice)
- [Sizing details](#sizing-details)
- [Current limitations](#current-limitations)

---

## Mental model

Lattice never owns a tree. The host owns the tree (a DOM, a Taffy layout tree, a test fixture)
and implements [`TableTree`] over it. Lattice then:

1. **Reads** the structure and CSS via the trait (`children`, `table_role`, `css_length`, …).
2. **Computes** a full table layout in memory.
3. **Writes** the result back, one node at a time, via [`TableTree::set_layout`].

The single entry point is:

```rust
pub fn compute_table_layout<T: TableTree>(
    tree: &mut T,
    table_node: T::NodeId,
    available_width: f32,
    available_height: Option<f32>,
) -> anyhow::Result<(f32, f32)>;
```

It returns the `(width, height)` border-box size the table occupies. The caller is responsible
for placing the table node itself within the surrounding flow; lattice positions everything
*inside* the table.

---

## The `TableTree` trait

This is the entire contract between lattice and the host. The host implements it over its own
node type.

```rust
pub trait TableTree {
    type NodeId: Copy + Clone + Eq + Hash + Debug;

    /// Children of `id` in document order.
    fn children(&self, id: Self::NodeId) -> Vec<Self::NodeId>;

    /// CSS `display` role this node plays in the table.
    fn table_role(&self, id: Self::NodeId) -> TableRole;

    /// A resolved CSS length value for a given property.
    fn css_length(&self, id: Self::NodeId, prop: CssProp) -> CssLength;

    /// An HTML attribute parsed as usize — used for `colspan` / `rowspan`.
    fn attr_usize(&self, id: Self::NodeId, attr: &str) -> Option<usize>;

    /// Write the computed layout for `id` back to the tree.
    fn set_layout(&mut self, id: Self::NodeId, layout: CellLayout);

    /// Lay out the children of cell `id` for the given inner content width,
    /// returning the content height they occupy.
    fn layout_cell(&mut self, id: Self::NodeId, available_width: f32) -> f32;

    /// Natural (pre-pass) border-box width of a cell, used to distribute auto
    /// column widths proportionally to content. Defaults to `0.0`.
    fn cell_content_width(&self, _id: Self::NodeId) -> f32 { 0.0 }
}
```

A few things worth calling out:

- **`layout_cell` is the recursion hook.** Lattice does not lay out the *contents* of a cell —
  block/flex/inline layout is the host's job. When lattice needs a cell's height, it calls
  `layout_cell` with the resolved inner width and the host runs its normal layout engine on the
  cell's subtree, returning the resulting content height. This is how lattice cooperates with
  Taffy rather than replacing it.
- **`css_length` returns already-resolved values.** The host is responsible for cascade,
  inheritance and unit parsing; lattice only sees [`CssLength`] (`Auto` / `Px` / `Percent` /
  `Zero`). For keyword-valued properties (`table-layout`, `border-collapse`) lattice uses a
  sentinel convention — see [Public types](#public-types).
- **`cell_content_width` is optional.** Mock/test trees can return `0.0`, in which case auto
  columns fall back to equal-width distribution.

---

## The layout pipeline

`compute_table_layout` runs these phases in order:

```
                build_model            build_section_grid         compute_column_widths
  table_node ───────────────▶ TableModel ───────────────▶ SectionGrid ──┐
                (model.rs)              (grid.rs)                        │ (sizing/columns.rs)
                                                                         ▼
                                                                   col_widths
                                                                         │
                          compute_row_heights                            │
            SectionGrid ───────────────────▶ row_heights ◀───────────────┘
                          (sizing/rows.rs, calls layout_cell)
                                                                         │
                          place groups → rows → cells                    ▼
                          (compute.rs) ──────────────────▶ set_layout(node, CellLayout)
```

1. **Build the model** (`model.rs`). Walk the tree from the table node and classify children by
   [`TableRole`]. Generate **anonymous boxes** where the spec requires them (CSS 2.1 §17.2.1):
   a bare row directly under the table becomes an anonymous row group; a bare cell under a row
   group becomes an anonymous row. Sections are bucketed into header / body / footer groups.

2. **Build per-section grids** (`grid.rs`). Run the slot-filling algorithm (HTML §4.9.11) to
   place each cell at a concrete `(row, col)` position, honoring `colspan` and `rowspan`.
   `rowspan` is **clamped to the section** — a span can never cross a `thead`/`tbody`/`tfoot`
   boundary.

3. **Compute column widths** (`sizing/columns.rs`). Determined once across *all* sections (a
   table has one set of columns). Explicit `width`/`%` cells pin their columns; the remaining
   space is distributed to auto columns proportionally to content width — see
   [Sizing details](#sizing-details).

4. **Compute row heights** (`sizing/rows.rs`). For each non-spanning cell, call `layout_cell`
   with the resolved inner width to get its content height, take the max with any explicit CSS
   `height`, add border + padding, and let the tallest cell set the row height.

5. **Place everything** (`compute.rs`). Sections are laid out **header → body → footer**
   regardless of source order. Each group is positioned relative to the table, each row relative
   to its group, each cell relative to its row, with `border-spacing` gutters inserted between
   and around tracks. Results are written via `set_layout`.

---

## Public types

Re-exported from the crate root (`gosub_lattice::*`):

| Type | Purpose |
|------|---------|
| [`TableTree`] | The host-implemented trait; the entire integration surface. |
| `compute_table_layout` | The entry-point function. |
| `TableRole` | What `display` role a node plays (`Table`, `Row`, `Cell`, `HeaderGroup`, …). |
| `CssLength` | A resolved length: `Auto` / `Px(f32)` / `Percent(f32)` / `Zero`. |
| `CssProp` | The set of CSS properties lattice reads (`Width`, `PaddingTop`, `BorderCollapse`, …). |
| `CellLayout` | The computed result written back per node: `position`, `size`, `border`, `padding`. |
| `BoxEdges` | Per-edge insets (`top`/`right`/`bottom`/`left`), used for border and padding. |
| `TableSizing` | `table-layout`: `Auto` (content-driven) or `Fixed`. |
| `BorderCollapse` | `border-collapse`: `Separate` or `Collapse`. |

### Keyword-property sentinel convention

Lattice's `css_length` returns a `CssLength`, but several table properties are keyword-valued
(`table-layout: fixed`, `border-collapse: collapse`). The host signals these by returning
`CssLength::Px(1.0)` as a sentinel:

| Property | `Px(1.0)` means | otherwise |
|----------|-----------------|-----------|
| `CssProp::TableLayout` | `TableSizing::Fixed` | `TableSizing::Auto` |
| `CssProp::BorderCollapse` | `BorderCollapse::Collapse` | `BorderCollapse::Separate` |

`border-spacing` defaults to `2.0` px on each axis when not a definite length.

---

## Coordinate system

All positions in [`CellLayout`] are **relative to the node's parent in the table hierarchy**,
not absolute:

- a group's position is relative to the **table**,
- a row's position is relative to its **group**,
- a cell's position is relative to its **row**.

Sizes are **border-box** (content + padding + border). The host is responsible for accumulating
these relative offsets into absolute coordinates if its rendering model needs them. (For
example, `gosub_render_pipeline` walks the table and adds each ancestor's offset to produce
absolute box positions.)

---

## Integrating lattice

To drive lattice, implement [`TableTree`] over your tree and call `compute_table_layout`. The
real integration in this workspace is `gosub_taffy`'s `LayoutDocument`, which:

- maps its DOM `display` strings to `TableRole`,
- maps `CssProp` variants to CSS property names and resolves them to `CssLength`,
- implements `layout_cell` by running Taffy's block layout on the cell subtree,
- implements `set_layout` by writing a `taffy::Layout` (location, size, border, padding).

A minimal sketch:

```rust
use gosub_lattice::{
    compute_table_layout, CellLayout, CssLength, CssProp, TableRole, TableTree,
};

struct MyTree { /* ... */ }

impl TableTree for MyTree {
    type NodeId = usize;

    fn children(&self, id: usize) -> Vec<usize> { /* ... */ }
    fn table_role(&self, id: usize) -> TableRole { /* map your `display` */ }
    fn css_length(&self, id: usize, prop: CssProp) -> CssLength { /* resolve */ }
    fn attr_usize(&self, id: usize, attr: &str) -> Option<usize> { /* colspan/rowspan */ }
    fn set_layout(&mut self, id: usize, layout: CellLayout) { /* store it */ }
    fn layout_cell(&mut self, id: usize, available_width: f32) -> f32 {
        // run your normal layout on the cell's children, return their height
        0.0
    }
}

let (w, h) = compute_table_layout(&mut tree, table_id, viewport_width, None)?;
```

The crate ships a `mock` module (used by the test suite) and a `table_console` binary that
prints a computed layout to the terminal — both are useful references for a from-scratch
implementation.

---

## Sizing details

**Column widths** (`sizing/columns.rs`) use a content-aware heuristic rather than naive equal
splitting:

1. Subtract the `border-spacing` gutters from the table width to get the available space.
2. Scan the first non-empty row. Single-column cells with an explicit `width`/`%` pin their
   column (never below the cell's min-content width, so a fixed-width cell can't clip its
   content). Their natural content widths are recorded.
3. Distribute the remaining space to auto columns:
   - **Narrow** columns (intrinsic width < 50 px — rank numbers, icons, buttons) get their
     natural width with a 14 px floor so they stay visible.
   - **Wide/content** columns share the rest proportionally to their natural widths.
   - If there's no content-width information (e.g. mock trees), fall back to equal distribution.

**Row heights** (`sizing/rows.rs`) take the max over each row's non-spanning cells of
`max(layout_cell height, explicit CSS height) + border + padding`. Explicit `height` acts as a
minimum — content can always grow the row taller.

---

## Current limitations

The engine implements the common, practical subset of CSS table layout. Known gaps:

- **`rowspan` height distribution** — cells with `rowspan > 1` are skipped when computing row
  heights; distributing a spanning cell's height across the rows it covers is a later phase.
- **`border-collapse: collapse`** — the property is parsed and modeled, but the separate-borders
  (`border-spacing`) model is what's fully wired through positioning.
- **`<col>` / `<colgroup>` width contributions** — column groups are parsed into the model but do
  not yet feed the column-width algorithm.
- **`caption-side` / `vertical-align`** — the properties are recognized but not yet applied to
  placement.
- **`table-layout: fixed`** — modeled (`TableSizing::Fixed`) but the auto algorithm is the path
  exercised in practice.

These are intentional staging points; the architecture (model → grid → sizing → placement)
leaves room to fill them in without reshaping the pipeline.
