//! Border-conflict resolution for `border-collapse: collapse` (CSS 2 §17.6.2.1, css-tables-3
//! §3.7.2).
//!
//! Every grid line is cut into segments, one per row or column it runs along. Each segment
//! takes the winner among the borders that meet on it: the cells on either side, their rows,
//! row groups, columns and column groups, and on the perimeter the table. A line inside a
//! spanning cell has no segment. Collapsed borders are centered on the grid line, so each
//! adjacent cell reserves and paints half of it.

use std::collections::HashMap;

use crate::grid::SectionGrid;
use crate::model::TableModel;
use crate::types::{BorderOwner, BorderStyle, BoxEdges, CollapsedBorders, CollapsedPaint, CssProp, EdgeSegment, Side};
use crate::TableTree;

/// Where a border comes from. Between two equally wide borders of the same style the earlier
/// origin wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    Cell,
    Row,
    RowGroup,
    Column,
    ColumnGroup,
    Table,
}

#[derive(Debug, Clone, Copy)]
struct Candidate<N> {
    width: f64,
    style: BorderStyle,
    origin: Origin,
    owner: BorderOwner<N>,
}

/// Whether `a` wins over `b`. A tie stays with `b`, which callers list first: further left or
/// further up.
fn beats<N>(a: &Candidate<N>, b: &Candidate<N>) -> bool {
    if b.style == BorderStyle::Hidden {
        return false;
    }
    if a.style == BorderStyle::Hidden {
        return true;
    }
    if a.width != b.width {
        return a.width > b.width;
    }
    if a.style != b.style {
        return a.style > b.style;
    }
    a.origin < b.origin
}

fn best<N: Copy>(candidates: impl IntoIterator<Item = Candidate<N>>) -> Option<Candidate<N>> {
    candidates.into_iter().fold(None, |acc, c| match acc {
        Some(b) if !beats(&c, &b) => Some(b),
        _ => Some(c),
    })
}

/// One resolved segment of a grid line.
#[derive(Debug, Clone, Copy)]
struct Segment<N> {
    /// Full resolved width; 0 when `hidden` won or nothing has a border.
    width: f64,
    /// Whether something paints the slot before (above/left) and after (below/right) the line:
    /// a cell, or the row of an empty slot. With nothing on one side, as on the perimeter or
    /// next to an anonymous empty row, the other side paints the whole border.
    painted: [bool; 2],
    /// The winning border, `None` when nothing is painted.
    winner: Option<Candidate<N>>,
}

/// A cell-edge segment in grid units: `track` is the column (top/bottom edges) or the
/// section row (left/right edges) it runs along.
#[derive(Debug, Clone, Copy)]
struct TrackSegment<N> {
    track: usize,
    width: f64,
    outset: f64,
    owner: Option<BorderOwner<N>>,
}

/// One cell's collapsed edges, before the tracks have sizes.
#[derive(Debug, Clone)]
pub(crate) struct CellEdges<N> {
    edges: [Vec<TrackSegment<N>>; 4],
    corners: [Option<BorderOwner<N>>; 4],
    corner_outsets: [[f64; 2]; 4],
}

impl<N: Copy + PartialEq> CellEdges<N> {
    /// The widest half on each edge: what the slot's border takes in layout.
    pub(crate) fn halves(&self) -> BoxEdges {
        let half = |e: usize| self.edges[e].iter().map(|s| s.width).fold(0.0, f64::max);
        BoxEdges {
            top: half(0),
            right: half(1),
            bottom: half(2),
            left: half(3),
        }
    }

    /// Turn tracks into offsets along each edge. `col_x`/`col_widths` and `row_y`/`row_heights`
    /// are the placed tracks; `first_col`/`first_row` are the cell's own first tracks.
    pub(crate) fn paint(
        &self,
        col_x: &[f64],
        col_widths: &[f64],
        row_y: &[f64],
        row_heights: &[f64],
        first_col: usize,
        first_row: usize,
    ) -> CollapsedPaint<N> {
        let span = |starts: &[f64], sizes: &[f64], first: usize, track: usize| {
            let origin = starts.get(first).copied().unwrap_or(0.0);
            let start = starts.get(track).copied().unwrap_or(origin);
            (
                start - origin,
                start + sizes.get(track).copied().unwrap_or(0.0) - origin,
            )
        };
        let edges = std::array::from_fn(|e| {
            let mut out: Vec<EdgeSegment<N>> = Vec::new();
            for seg in &self.edges[e] {
                let (start, end) = match Side::ALL[e] {
                    Side::Top | Side::Bottom => span(col_x, col_widths, first_col, seg.track),
                    Side::Left | Side::Right => span(row_y, row_heights, first_row, seg.track),
                };
                match out.last_mut() {
                    Some(prev) if prev.width == seg.width && prev.outset == seg.outset && prev.owner == seg.owner => {
                        prev.end = end;
                    }
                    _ => out.push(EdgeSegment {
                        start,
                        end,
                        width: seg.width,
                        outset: seg.outset,
                        owner: seg.owner,
                    }),
                }
            }
            out
        });
        CollapsedPaint {
            edges,
            corners: self.corners,
            corner_outsets: self.corner_outsets,
        }
    }
}

/// Result of [`resolve`]: the layout half-borders and the per-cell edges to paint.
pub(crate) struct Collapse<N> {
    pub borders: HashMap<N, CollapsedBorders>,
    pub cells: HashMap<N, CellEdges<N>>,
    /// Per row, the slots no cell covers, by column: the row paints their share of the lines
    /// around them as if they were cells.
    pub empty_slots: EmptySlots<N>,
}

/// Per row, its empty slots by column.
pub(crate) type EmptySlots<N> = HashMap<N, Vec<(usize, CellEdges<N>)>>;

#[derive(Clone, Copy)]
struct Track<N> {
    node: Option<N>,
    group: Option<N>,
    group_first: bool,
    group_last: bool,
}

/// Resolve every grid line of a collapsed table. `grids` and `groups` are in render order
/// (header -> body -> footer), one group node per grid (`None` for an anonymous group).
pub(crate) fn resolve<T: TableTree>(
    tree: &T,
    model: &TableModel<T::NodeId>,
    n_cols: usize,
    grids: &[&SectionGrid<T::NodeId>],
    groups: &[Option<T::NodeId>],
) -> Collapse<T::NodeId> {
    let n_rows: usize = grids.iter().map(|g| g.n_rows).sum();

    let mut slots: Vec<Vec<Option<T::NodeId>>> = vec![vec![None; n_cols]; n_rows];
    let mut rows: Vec<Track<T::NodeId>> = Vec::with_capacity(n_rows);
    let mut local_rows: Vec<usize> = Vec::with_capacity(n_rows);
    // (cell, first global row, section-local first row, first col, rowspan, colspan)
    let mut placed = Vec::new();
    for (grid, &group) in grids.iter().zip(groups) {
        let offset = rows.len();
        for r in 0..grid.n_rows {
            rows.push(Track {
                node: grid.row_nodes.get(r).copied().flatten(),
                group,
                group_first: r == 0,
                group_last: r + 1 == grid.n_rows,
            });
            local_rows.push(r);
        }
        for cell in grid.cells() {
            let col_end = (cell.col + cell.colspan).min(n_cols);
            let row_end = (cell.row + cell.rowspan).min(grid.n_rows);
            for slot_row in &mut slots[offset + cell.row..offset + row_end] {
                slot_row[cell.col..col_end].fill(Some(cell.node));
            }
            placed.push((
                cell.node,
                offset + cell.row,
                cell.row,
                cell.col,
                row_end - cell.row,
                col_end - cell.col,
            ));
        }
    }
    let cols = column_tracks(tree, model, n_cols);
    let painted: Vec<Vec<bool>> = slots
        .iter()
        .zip(&rows)
        .map(|(line, row)| line.iter().map(|slot| slot.is_some() || row.node.is_some()).collect())
        .collect();

    let candidate = |node: T::NodeId, side: Side, origin: Origin| {
        let prop = match side {
            Side::Top => CssProp::BorderTopWidth,
            Side::Right => CssProp::BorderRightWidth,
            Side::Bottom => CssProp::BorderBottomWidth,
            Side::Left => CssProp::BorderLeftWidth,
        };
        Candidate {
            width: tree.css_length(node, prop).px_or(0.0),
            style: tree.border_style(node, side),
            origin,
            owner: BorderOwner { node, side },
        }
    };
    let resolve_segment = |list: Vec<Candidate<T::NodeId>>, painted: [bool; 2]| {
        let winner = best(list).filter(|c| c.style != BorderStyle::Hidden && c.width > 0.0);
        Segment {
            width: winner.map_or(0.0, |c| c.width),
            painted,
            winner,
        }
    };

    // hseg[h][c]: horizontal line `h` (0 = table top) along column `c`; `None` inside a span.
    let mut hseg: Vec<Vec<Option<Segment<T::NodeId>>>> = vec![vec![None; n_cols]; n_rows + 1];
    for (h, line) in hseg.iter_mut().enumerate() {
        for (c, seg) in line.iter_mut().enumerate() {
            let above = if h > 0 { slots[h - 1][c] } else { None };
            let below = if h < n_rows { slots[h][c] } else { None };
            if h > 0 && h < n_rows && above.is_some() && above == below {
                continue;
            }
            let mut list = Vec::new();
            if let Some(cell) = above {
                list.push(candidate(cell, Side::Bottom, Origin::Cell));
            }
            if h > 0 {
                let row = rows[h - 1];
                list.extend(row.node.map(|n| candidate(n, Side::Bottom, Origin::Row)));
                if row.group_last {
                    list.extend(row.group.map(|n| candidate(n, Side::Bottom, Origin::RowGroup)));
                }
            } else {
                list.extend(cols[c].node.map(|n| candidate(n, Side::Top, Origin::Column)));
                list.extend(cols[c].group.map(|n| candidate(n, Side::Top, Origin::ColumnGroup)));
                list.push(candidate(model.node, Side::Top, Origin::Table));
            }
            if let Some(cell) = below {
                list.push(candidate(cell, Side::Top, Origin::Cell));
            }
            if h < n_rows {
                let row = rows[h];
                list.extend(row.node.map(|n| candidate(n, Side::Top, Origin::Row)));
                if row.group_first {
                    list.extend(row.group.map(|n| candidate(n, Side::Top, Origin::RowGroup)));
                }
            } else {
                list.extend(cols[c].node.map(|n| candidate(n, Side::Bottom, Origin::Column)));
                list.extend(cols[c].group.map(|n| candidate(n, Side::Bottom, Origin::ColumnGroup)));
                list.push(candidate(model.node, Side::Bottom, Origin::Table));
            }
            *seg = Some(resolve_segment(
                list,
                [h > 0 && painted[h - 1][c], h < n_rows && painted[h][c]],
            ));
        }
    }

    // vseg[v][r]: vertical line `v` (0 = table left) along global row `r`.
    let mut vseg: Vec<Vec<Option<Segment<T::NodeId>>>> = vec![vec![None; n_rows]; n_cols + 1];
    for (v, line) in vseg.iter_mut().enumerate() {
        for (r, seg) in line.iter_mut().enumerate() {
            let left = if v > 0 { slots[r][v - 1] } else { None };
            let right = if v < n_cols { slots[r][v] } else { None };
            if v > 0 && v < n_cols && left.is_some() && left == right {
                continue;
            }
            let row = rows[r];
            let mut list = Vec::new();
            if let Some(cell) = left {
                list.push(candidate(cell, Side::Right, Origin::Cell));
            }
            if v > 0 {
                let col = cols[v - 1];
                list.extend(col.node.map(|n| candidate(n, Side::Right, Origin::Column)));
                if col.group_last {
                    list.extend(col.group.map(|n| candidate(n, Side::Right, Origin::ColumnGroup)));
                }
            } else {
                list.extend(row.node.map(|n| candidate(n, Side::Left, Origin::Row)));
                list.extend(row.group.map(|n| candidate(n, Side::Left, Origin::RowGroup)));
                list.push(candidate(model.node, Side::Left, Origin::Table));
            }
            if let Some(cell) = right {
                list.push(candidate(cell, Side::Left, Origin::Cell));
            }
            if v < n_cols {
                let col = cols[v];
                list.extend(col.node.map(|n| candidate(n, Side::Left, Origin::Column)));
                if col.group_first {
                    list.extend(col.group.map(|n| candidate(n, Side::Left, Origin::ColumnGroup)));
                }
            } else {
                list.extend(row.node.map(|n| candidate(n, Side::Right, Origin::Row)));
                list.extend(row.group.map(|n| candidate(n, Side::Right, Origin::RowGroup)));
                list.push(candidate(model.node, Side::Right, Origin::Table));
            }
            *seg = Some(resolve_segment(
                list,
                [v > 0 && painted[r][v - 1], v < n_cols && painted[r][v]],
            ));
        }
    }

    // The joint where lines `h` and `v` cross goes to the strongest segment meeting there;
    // ties go to the one above, then the one to the left.
    let joint = |h: usize, v: usize| {
        let above = if h > 0 { vseg[v][h - 1] } else { None };
        let left = if v > 0 { hseg[h][v - 1] } else { None };
        let right = if v < n_cols { hseg[h][v] } else { None };
        let below = if h < n_rows { vseg[v][h] } else { None };
        best(
            [above, left, right, below]
                .into_iter()
                .flatten()
                .filter_map(|s| s.winner),
        )
        .map(|c| c.owner)
    };

    // The quarter of joint (`h`, `v`) that slot (`r`, `c`) paints reaches outside the slot as
    // far as the widest line through the joint, where nothing is painted across.
    let corner_outset = |h: usize, v: usize, r: usize, c: usize| {
        let other_c = if c >= v {
            v.checked_sub(1)
        } else {
            Some(v).filter(|&v| v < n_cols)
        };
        let other_r = if r >= h {
            h.checked_sub(1)
        } else {
            Some(h).filter(|&h| h < n_rows)
        };
        let vertical = [h.checked_sub(1), Some(h).filter(|&h| h < n_rows)]
            .into_iter()
            .flatten()
            .filter_map(|r| vseg[v][r])
            .map(|s| s.width)
            .fold(0.0, f64::max);
        let horizontal = [v.checked_sub(1), Some(v).filter(|&v| v < n_cols)]
            .into_iter()
            .flatten()
            .filter_map(|c| hseg[h][c])
            .map(|s| s.width)
            .fold(0.0, f64::max);
        [
            if other_c.is_some_and(|oc| painted[r][oc]) {
                0.0
            } else {
                vertical / 2.0
            },
            if other_r.is_some_and(|or| painted[or][c]) {
                0.0
            } else {
                horizontal / 2.0
            },
        ]
    };

    // `across` is the side of the line away from the cell: 0 for its top/left edges.
    let track_segment = |seg: Option<Segment<T::NodeId>>, track: usize, across: usize| {
        let seg = seg.unwrap_or(Segment {
            width: 0.0,
            painted: [true; 2],
            winner: None,
        });
        TrackSegment {
            track,
            width: seg.width / 2.0,
            outset: if seg.painted[across] { 0.0 } else { seg.width / 2.0 },
            owner: seg.winner.map(|c| c.owner),
        }
    };

    let mut out = Collapse {
        borders: HashMap::new(),
        cells: HashMap::new(),
        empty_slots: HashMap::new(),
    };
    for (r, row) in rows.iter().enumerate() {
        let Some(row_node) = row.node else { continue };
        for c in (0..n_cols).filter(|&c| slots[r][c].is_none()) {
            let local_row = local_rows[r];
            let edges = [
                vec![track_segment(hseg[r][c], c, 0)],
                vec![track_segment(vseg[c + 1][r], local_row, 1)],
                vec![track_segment(hseg[r + 1][c], c, 1)],
                vec![track_segment(vseg[c][r], local_row, 0)],
            ];
            let corners = [joint(r, c), joint(r, c + 1), joint(r + 1, c + 1), joint(r + 1, c)];
            let corner_outsets = [
                corner_outset(r, c, r, c),
                corner_outset(r, c + 1, r, c),
                corner_outset(r + 1, c + 1, r, c),
                corner_outset(r + 1, c, r, c),
            ];
            out.empty_slots.entry(row_node).or_default().push((
                c,
                CellEdges {
                    edges,
                    corners,
                    corner_outsets,
                },
            ));
        }
    }
    for (node, g0, local_row, c0, rowspan, colspan) in placed {
        let (g1, c1) = (g0 + rowspan, c0 + colspan);
        let edges: [Vec<TrackSegment<T::NodeId>>; 4] = [
            (c0..c1).map(|c| track_segment(hseg[g0][c], c, 0)).collect(),
            (g0..g1)
                .map(|g| track_segment(vseg[c1][g], local_row + g - g0, 1))
                .collect(),
            (c0..c1).map(|c| track_segment(hseg[g1][c], c, 1)).collect(),
            (g0..g1)
                .map(|g| track_segment(vseg[c0][g], local_row + g - g0, 0))
                .collect(),
        ];
        let cell = CellEdges {
            edges,
            corners: [joint(g0, c0), joint(g0, c1), joint(g1, c1), joint(g1, c0)],
            corner_outsets: [
                corner_outset(g0, c0, g0, c0),
                corner_outset(g0, c1, g0, c1 - 1),
                corner_outset(g1, c1, g1 - 1, c1 - 1),
                corner_outset(g1, c0, g1 - 1, c0),
            ],
        };
        out.borders.insert(node, CollapsedBorders { layout: cell.halves() });
        out.cells.insert(node, cell);
    }
    out
}

type ColumnAt<N> = (Option<N>, Option<N>);

/// The column element and column group behind each column, expanded by `span` like
/// [`crate::sizing::columns::column_specs`]. A column group without columns has no column
/// element of its own.
fn column_tracks<T: TableTree>(tree: &T, model: &TableModel<T::NodeId>, n_cols: usize) -> Vec<Track<T::NodeId>> {
    // (column, group) per column: no column for a group without columns, no group for an anonymous one.
    let mut expanded: Vec<ColumnAt<T::NodeId>> = Vec::new();
    for group in &model.column_groups {
        if let (true, Some(node)) = (group.columns.is_empty(), group.node) {
            let span = tree.attr_usize(node, "span").unwrap_or(1).max(1);
            expanded.extend(std::iter::repeat_n((None, group.node), span));
        }
        for &col in &group.columns {
            let span = tree.attr_usize(col, "span").unwrap_or(1).max(1);
            expanded.extend(std::iter::repeat_n((Some(col), group.node), span));
        }
    }
    let group_at = |c: usize| expanded.get(c).map(|&(_, g)| g);
    (0..n_cols)
        .map(|c| Track {
            node: expanded.get(c).and_then(|&(n, _)| n),
            group: group_at(c).flatten(),
            group_first: c == 0 || group_at(c - 1) != group_at(c),
            group_last: group_at(c + 1) != group_at(c),
        })
        .collect()
}
