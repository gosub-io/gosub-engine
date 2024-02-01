use lazy_static::lazy_static;
use std::sync::Mutex;
use thiserror::Error;

use crate::web_executor::js::v8::V8Engine;
pub use compile::*;
pub use context::*;
pub use function::*;
pub use object::*;
pub use runtime::*;
pub use value::*;
pub use value_conversion::*;

use crate::types::Result;

mod compile;
mod context;
mod function;
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
    type Value: JSValue;

    type Index;

    fn get<T: Into<Self::Index>>(&self, index: T) -> Result<Self::Value>;

    fn set<T: Into<Self::Index>>(&self, index: T, value: &Self::Value) -> Result<()>;

    fn push(&self, value: Self::Value) -> Result<()>;

    fn pop(&self) -> Result<Self::Value>;

    fn remove<T: Into<Self::Index>>(&self, index: T) -> Result<()>;

    fn length(&self) -> Result<Self::Index>;

    //TODO: implement other things when needed. Maybe also `Iterator`?
}

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
