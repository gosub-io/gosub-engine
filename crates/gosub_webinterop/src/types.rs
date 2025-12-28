pub use args::*;
pub use field::*;
pub use generics::*;
pub use primitive::*;
pub use slice::*;
pub use ty::*;
mod args;
pub mod executor;
mod field;
mod generics;
mod primitive;
mod slice;
mod ty;

// #[derive(Clone, PartialEq, Debug)]
// pub(crate) struct PropertyOptions {
//     pub(crate) executor: Executor,
//     pub(crate) rename: Option<String>,
// }
//TODO: is this still needed?
