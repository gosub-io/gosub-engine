use std::sync::Mutex;

use lazy_static::lazy_static;
use thiserror::Error;

pub use compile::*;
pub use context::*;
pub use function::*;
use gosub_shared::types::Result;
pub use interop::*;
pub use object::*;
pub use runtime::*;
pub use value::*;
pub use value_conversion::*;

use crate::js::v8::V8Engine;

mod compile;
mod context;
mod function;
mod interop;
mod object;
mod runtime;
pub mod v8;
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
}

lazy_static! {
    pub static ref RUNTIME: Mutex<V8Engine<'static>> = Mutex::new(V8Engine::new());
}

pub trait JSArray {
    type RT: JSRuntime;

    fn get(
        &self,
        index: <Self::RT as JSRuntime>::ArrayIndex,
    ) -> Result<<Self::RT as JSRuntime>::Value>;

    fn set(
        &self,
        index: <Self::RT as JSRuntime>::ArrayIndex,
        value: &<Self::RT as JSRuntime>::Value,
    ) -> Result<()>;

    fn push(&self, value: <Self::RT as JSRuntime>::Value) -> Result<()>;

    fn pop(&self) -> Result<<Self::RT as JSRuntime>::Value>;

    fn remove<T: Into<<Self::RT as JSRuntime>::ArrayIndex>>(&self, index: T) -> Result<()>;

    fn length(&self) -> Result<<Self::RT as JSRuntime>::ArrayIndex>;

    //TODO: implement other things when needed. Maybe also `Iterator`?
}

#[derive(Debug, Clone, PartialEq)]
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
