use crate::js::{JSRuntime, JSValue};
use gosub_shared::types::Result;

pub trait JSArray: Iterator {
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

    fn len(&self) -> <Self::RT as JSRuntime>::ArrayIndex;

    fn is_empty(&self) -> bool;

    fn new(
        ctx: <Self::RT as JSRuntime>::Context,
        cap: <Self::RT as JSRuntime>::ArrayIndex,
    ) -> Result<Self>
    where
        Self: Sized;

    fn new_with_data(
        ctx: <Self::RT as JSRuntime>::Context,
        data: &[<Self::RT as JSRuntime>::Value],
    ) -> Result<Self>
    where
        Self: Sized;
}
