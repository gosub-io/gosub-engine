use gosub_shared::types::Result;

use crate::js::{AsArray, IntoWebValue, JSType, WebRuntime};

pub trait WebValue:
    Sized + From<<Self::RT as WebRuntime>::Object> + From<<Self::RT as WebRuntime>::Array> + AsArray<Runtime = Self::RT>
where
    Self: Sized,
{
    type RT: WebRuntime<Value = Self>;

    fn as_string(&self) -> Result<String>;

    fn as_number(&self) -> Result<f64>;

    fn as_bool(&self) -> Result<bool>;

    fn as_object(&self) -> Result<<Self::RT as WebRuntime>::Object>;

    fn as_array(&self) -> Result<<Self::RT as WebRuntime>::Array>;

    fn is_string(&self) -> bool;

    fn is_number(&self) -> bool;

    fn is_bool(&self) -> bool;

    fn is_object(&self) -> bool;

    fn is_array(&self) -> bool;

    fn is_null(&self) -> bool;

    fn is_undefined(&self) -> bool;

    fn is_function(&self) -> bool;

    fn type_of(&self) -> JSType;

    fn new_object(ctx: <Self::RT as WebRuntime>::Context) -> Result<<Self::RT as WebRuntime>::Object>;

    fn new_array<T: IntoWebValue<Self, Value = Self>>(
        ctx: <Self::RT as WebRuntime>::Context,
        value: &[T],
    ) -> Result<<Self::RT as WebRuntime>::Array>;

    fn new_empty_array(ctx: <Self::RT as WebRuntime>::Context) -> Result<<Self::RT as WebRuntime>::Array>;

    fn new_string(ctx: <Self::RT as WebRuntime>::Context, value: &str) -> Result<Self>;

    fn new_number<N: Into<f64>>(context: <Self::RT as WebRuntime>::Context, value: N) -> Result<Self>;

    fn new_bool(ctx: <Self::RT as WebRuntime>::Context, value: bool) -> Result<Self>;

    fn new_null(ctx: <Self::RT as WebRuntime>::Context) -> Result<Self>;

    fn new_undefined(ctx: <Self::RT as WebRuntime>::Context) -> Result<Self>;
}
