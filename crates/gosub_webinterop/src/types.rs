pub(crate) use args::*;
pub(crate) use field::*;
pub(crate) use generics::*;
pub(crate) use primitive::*;
pub(crate) use slice::*;
pub(crate) use ty::*;
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
