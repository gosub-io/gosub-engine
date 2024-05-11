pub mod node;
pub mod text;

/// Numerical values that map `rendertree::NodeType` to C
#[repr(C)]
pub enum CNodeType {
    Root = 0,
    Text = 1,
}
