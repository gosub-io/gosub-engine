use gosub_shared::types::Result;

use crate::js::{AsArray, JSRuntime};

pub trait JSArray:
    Iterator + Into<<Self::RT as JSRuntime>::Value> + AsArray<Runtime = Self::RT>
{
    type RT: JSRuntime<Array = Self>;

    fn get(&self, index: usize) -> Result<<Self::RT as JSRuntime>::Value>;

    fn set(&self, index: usize, value: &<Self::RT as JSRuntime>::Value) -> Result<()>;

    fn push(&self, value: <Self::RT as JSRuntime>::Value) -> Result<()>;

    fn pop(&self) -> Result<<Self::RT as JSRuntime>::Value>;

    fn remove(&self, index: usize) -> Result<()>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool;

    fn new(ctx: <Self::RT as JSRuntime>::Context, cap: usize) -> Result<Self>
    where
        Self: Sized;

    fn new_with_data(
        ctx: <Self::RT as JSRuntime>::Context,
        data: &[<Self::RT as JSRuntime>::Value],
    ) -> Result<Self>
    where
        Self: Sized;

    fn as_value(&self) -> <Self::RT as JSRuntime>::Value;

    fn as_vec(&self) -> Vec<<Self::RT as JSRuntime>::Value>;
}
