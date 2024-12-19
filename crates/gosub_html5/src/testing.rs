//! Testing harness and utilities for testing the engine
pub mod tokenizer;
pub mod tree_construction;

pub const FIXTURE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/./tests/data/html5lib-tests",);
pub const TREE_CONSTRUCTION_PATH: &str = "tree-construction";
