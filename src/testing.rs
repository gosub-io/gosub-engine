use crate::html5::{
    node::{NodeData, NodeId},
    parser::document::{Document, DocumentHandle},
};
use std::fmt::Display;

pub mod tokenizer;
pub mod tree_construction;

pub const FIXTURE_ROOT: &str = "./tests/data/html5lib-tests";

/// Render a document handle in the format used in the tree construction tests under
/// ./tests/data/html5lib-tests/tree-construction/.
pub struct TreeConstructionFormat(DocumentHandle);

impl Display for TreeConstructionFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn print_node(
            f: &mut std::fmt::Formatter<'_>,
            node_id: NodeId,
            document_offset_id: isize,
            indent: isize,
            doc: &Document,
        ) -> isize {
            let mut next_expected_idx = document_offset_id;
            let node = doc.get_node_by_id(node_id).unwrap();

            match &node.data {
                NodeData::Document(..) => {}

                NodeData::Element(element) => {
                    writeln!(
                        f,
                        "|{}<{}>",
                        " ".repeat(indent as usize * 2 + 1),
                        element.name()
                    )
                    .unwrap();

                    // Check attributes if any
                    for (name, value) in element.attributes.iter() {
                        writeln!(
                            f,
                            "|{}{name}=\"{value}\"",
                            " ".repeat((indent as usize * 2) + 3),
                        )
                        .unwrap();
                    }
                }

                NodeData::Text(text) => {
                    writeln!(
                        f,
                        "|{}\"{}\"",
                        " ".repeat(indent as usize * 2 + 1),
                        text.value()
                    )
                    .unwrap();
                }

                NodeData::Comment(comment) => {
                    writeln!(
                        f,
                        "|{}<!-- {} -->\n",
                        " ".repeat(indent as usize * 2 + 1),
                        comment.value()
                    )
                    .unwrap();
                }
            }

            for &child_id in &node.children {
                next_expected_idx = print_node(f, child_id, document_offset_id, indent + 1, doc);
            }

            next_expected_idx
        }

        print_node(f, NodeId::root(), 0, -1, &self.0.get());

        Ok(())
    }
}

impl DocumentHandle {
    pub fn tree_construction_format(&self) -> TreeConstructionFormat {
        TreeConstructionFormat(Document::clone(self))
    }
}
