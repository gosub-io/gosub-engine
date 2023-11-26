pub mod text;

/// Numerical values that map render_tree::NodeType to C
#[repr(C)]
pub enum CNodeType {
    Root = 0,
    Text = 1,
}
