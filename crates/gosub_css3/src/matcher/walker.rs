use crate::matcher::syntax_matcher::CssSyntaxTree;

pub struct MatchWalker {
    tree: CssSyntaxTree,
}

impl MatchWalker {
    pub fn new(tree: &CssSyntaxTree) -> Self {
        Self { tree: tree.clone() }
    }

    // pub fn walk(&self) {
    //     let _ = self.inner_walk(&self.tree);
    // }
    //
    // fn inner_walk(&self, node: &Node) -> Result<(), std::io::Error> {
    //     match node.node_type.deref() {
    //         NodeType::StyleSheet { children } => {
    //             for child in children.iter() {
    //                 self.inner_walk(child)?;
    //             }
    //         }
    //         NodeType::Rule { block, .. } => {
    //             if block.is_some() {
    //                 self.inner_walk(block.as_ref().unwrap())?;
    //             }
    //         }
    //         NodeType::AtRule { block, .. } => {
    //             if block.is_some() {
    //                 self.inner_walk(block.as_ref().unwrap())?;
    //             }
    //         }
    //         NodeType::Block { children, .. } => {
    //             for child in children.iter() {
    //                 self.inner_walk(child)?;
    //             }
    //         }
    //         NodeType::Declaration { property, value, .. } => {
    //             println!("Matching property '{}' against value '{:?}'", property, value);
    //
    //
    //         }
    //         _ => {}
    //     }
    //
    //     Ok(())
    // }
}
