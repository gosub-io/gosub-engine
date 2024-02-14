use gosub_shared::types::Result;
use crate::web_executor::js::{JSArray, JSContext, JSObject, JSRuntime, JSType};

pub trait JSValue
where
    Self: Sized,
{
    type RT: JSRuntime;

    fn as_string(&self) -> Result<String>;

    fn as_number(&self) -> Result<f64>;

    fn as_bool(&self) -> Result<bool>;

    fn as_object(&self) -> Result<<Self::RT as JSRuntime>::Object>;

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

    fn new_string(ctx: <Self::RT as JSRuntime>::Context, value: &str) -> Result<Self>;

    fn new_number<N: Into<f64>>(
        context: <Self::RT as JSRuntime>::Context,
        value: N,
    ) -> Result<Self>;

    fn new_bool(ctx: <Self::RT as JSRuntime>::Context, value: bool) -> Result<Self>;

    fn new_null(ctx: <Self::RT as JSRuntime>::Context) -> Result<Self>;

    fn new_undefined(ctx: <Self::RT as JSRuntime>::Context) -> Result<Self>;
}
