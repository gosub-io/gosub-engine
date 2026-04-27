use gosub_engine::tab::TabId;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Copy, Clone, Debug)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Debug)]
pub(crate) enum LayoutNode {
    Leaf(TabId),
    Rows(Vec<LayoutNode>),
    Cols(Vec<LayoutNode>),
}

pub(crate) type LayoutHandle = Rc<RefCell<LayoutNode>>;

// Compute (tab, rect) pairs for all visible leaves
pub(crate) fn compute_layout(node: &LayoutNode, rect: Rect, out: &mut Vec<(TabId, Rect)>) {
    match node {
        LayoutNode::Leaf(tid) => out.push((*tid, rect)),
        LayoutNode::Rows(children) => {
            let n = children.len().max(1) as i32;
            let h_each = rect.h / n;
            let mut y = rect.y;
            for (i, ch) in children.iter().enumerate() {
                let h = if i == children.len() - 1 {
                    rect.y + rect.h - y
                } else {
                    h_each
                };
                compute_layout(
                    ch,
                    Rect {
                        x: rect.x,
                        y,
                        w: rect.w,
                        h,
                    },
                    out,
                );
                y += h_each;
            }
        }
        LayoutNode::Cols(children) => {
            let n = children.len().max(1) as i32;
            let w_each = rect.w / n;
            let mut x = rect.x;
            for (i, ch) in children.iter().enumerate() {
                let w = if i == children.len() - 1 {
                    rect.x + rect.w - x
                } else {
                    w_each
                };
                compute_layout(
                    ch,
                    Rect {
                        x,
                        y: rect.y,
                        w,
                        h: rect.h,
                    },
                    out,
                );
                x += w_each;
            }
        }
    }
}

// Find the leaf at a point (window coords)
pub(crate) fn find_leaf_at(node: &LayoutNode, rect: Rect, px: f64, py: f64) -> Option<TabId> {
    match node {
        LayoutNode::Leaf(tid) => {
            let pxi = px as i32;
            let pyi = py as i32;

            if pxi >= rect.x && pxi < rect.x + rect.w && pyi >= rect.y && pyi < rect.y + rect.h {
                Some(*tid)
            } else {
                None
            }
        }
        LayoutNode::Rows(children) => {
            let n = children.len().max(1) as i32;
            let h_each = rect.h / n;
            let mut y = rect.y;
            for (i, ch) in children.iter().enumerate() {
                let h = if i == children.len() - 1 {
                    rect.y + rect.h - y
                } else {
                    h_each
                };
                if let Some(t) = find_leaf_at(
                    ch,
                    Rect {
                        x: rect.x,
                        y,
                        w: rect.w,
                        h,
                    },
                    px,
                    py,
                ) {
                    return Some(t);
                }
                y += h_each;
            }
            None
        }
        LayoutNode::Cols(children) => {
            let n = children.len().max(1) as i32;
            let w_each = rect.w / n;
            let mut x = rect.x;
            for (i, ch) in children.iter().enumerate() {
                let w = if i == children.len() - 1 {
                    rect.x + rect.w - x
                } else {
                    w_each
                };
                if let Some(t) = find_leaf_at(
                    ch,
                    Rect {
                        x,
                        y: rect.y,
                        w,
                        h: rect.h,
                    },
                    px,
                    py,
                ) {
                    return Some(t);
                }
                x += w_each;
            }
            None
        }
    }
}

// Replace a leaf with Cols(leaf + new tabs)
pub(crate) fn split_leaf_into_cols(root: &LayoutHandle, target: TabId, new_tabs: Vec<TabId>) -> bool {
    fn rec(n: &mut LayoutNode, target: TabId, new_tabs: &[TabId]) -> bool {
        match n {
            LayoutNode::Leaf(t) if *t == target => {
                let mut children = Vec::with_capacity(1 + new_tabs.len());
                children.push(LayoutNode::Leaf(*t));
                for &nt in new_tabs {
                    children.push(LayoutNode::Leaf(nt));
                }
                *n = LayoutNode::Cols(children);
                true
            }
            LayoutNode::Rows(v) | LayoutNode::Cols(v) => {
                for ch in v {
                    if rec(ch, target, new_tabs) {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
    rec(&mut root.borrow_mut(), target, &new_tabs)
}

// Replace a leaf with Rows(leaf + new tabs)
pub(crate) fn split_leaf_into_rows(root: &LayoutHandle, target: TabId, new_tabs: Vec<TabId>) -> bool {
    fn rec(n: &mut LayoutNode, target: TabId, new_tabs: &[TabId]) -> bool {
        match n {
            LayoutNode::Leaf(t) if *t == target => {
                let mut children = Vec::with_capacity(1 + new_tabs.len());
                children.push(LayoutNode::Leaf(*t));
                for &nt in new_tabs {
                    children.push(LayoutNode::Leaf(nt));
                }
                *n = LayoutNode::Rows(children);
                true
            }
            LayoutNode::Rows(v) | LayoutNode::Cols(v) => {
                for ch in v {
                    if rec(ch, target, new_tabs) {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
    rec(&mut root.borrow_mut(), target, &new_tabs)
}

// Close a leaf; collapse single-child containers
pub(crate) fn close_leaf(root: &LayoutHandle, target: TabId) -> bool {
    fn rec(n: &mut LayoutNode, target: TabId) -> bool {
        match n {
            LayoutNode::Leaf(t) => *t == target,
            LayoutNode::Rows(v) => {
                v.retain_mut(|ch| !rec(ch, target));
                // Collapse
                if v.len() == 1 {
                    *n = v.remove(0);
                }
                false
            }
            LayoutNode::Cols(v) => {
                v.retain_mut(|ch| !rec(ch, target));
                if v.len() == 1 {
                    *n = v.remove(0);
                }
                false
            }
        }
    }
    let mut root_b = root.borrow_mut();
    match &*root_b {
        LayoutNode::Leaf(t) if *t == target => false, // don't allow removing the last pane here
        _ => {
            rec(&mut root_b, target);
            true
        }
    }
}

// Collect all leaves (TabIds)
pub(crate) fn collect_leaves(node: &LayoutNode, out: &mut Vec<TabId>) {
    match node {
        LayoutNode::Leaf(t) => out.push(*t),
        LayoutNode::Rows(v) | LayoutNode::Cols(v) => v.iter().for_each(|c| collect_leaves(c, out)),
    }
}
