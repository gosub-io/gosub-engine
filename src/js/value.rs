use crate::js::{JSArray, JSContext, JSObject, JSType};
use crate::types::Result;

pub trait JSValue
where
    Self: Sized,
{
    type Object: JSObject;

    type Context: JSContext;

    fn as_string(&self) -> Result<String>;

    fn as_number(&self) -> Result<f64>;

    fn as_bool(&self) -> Result<bool>;

    fn as_object(&self) -> Result<Self::Object>;

    fn is_string(&self) -> bool;

    fn is_number(&self) -> bool;

    fn is_bool(&self) -> bool;

    fn is_object(&self) -> bool;

    fn is_array(&self) -> bool;

    fn is_null(&self) -> bool;

    fn is_undefined(&self) -> bool;

    fn is_function(&self) -> bool;

    fn type_of(&self) -> JSType;

    // fn new_object() -> Result<Self::Object>;
    //
    // fn new_array<T: ValueConversion<Self>>(value: &[T]) -> Result<Self::Array>;

    fn new_string(ctx: Self::Context, value: &str) -> Result<Self>;

    fn new_number<N: Into<f64>>(context: Self::Context, value: N) -> Result<Self>;

    fn new_bool(ctx: Self::Context, value: bool) -> Result<Self>;

    fn new_null(ctx: Self::Context) -> Result<Self>;

    fn new_undefined(ctx: Self::Context) -> Result<Self>;
}
