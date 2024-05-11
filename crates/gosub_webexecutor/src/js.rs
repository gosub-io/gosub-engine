use thiserror::Error;

pub use array::*;
pub use compile::*;
pub use context::*;
pub use function::*;
pub use interop::*;
pub use object::*;
pub use runtime::*;
pub use value::*;
pub use value_conversion::*;

mod array;
mod compile;
mod context;
mod function;
mod interop;
mod object;
mod runtime;
mod value;
mod value_conversion;

#[derive(Error, Debug)]
pub enum JSError {
    #[error("generic error: {0}")]
    Generic(String),

    #[error("conversion error: {0}")]
    Conversion(String),

    #[error("runtime error: {0}")]
    Runtime(String),

    #[error("compile error: {0}")]
    Compile(String),

    #[error("initialize error: {0}")]
    Initialize(String),

    #[error("execution error: {0}")]
    Execution(String),

    #[error("exception: {0}")]
    Exception(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JSType {
    Undefined,
    Null,
    Boolean,
    Number,
    String,
    Object,
    Array,
    Function,
    Other(String),
}
