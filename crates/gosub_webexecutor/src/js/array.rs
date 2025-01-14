use gosub_shared::types::Result;

use crate::js::{AsArray, WebRuntime};

pub trait WebArray: Iterator + Into<<Self::RT as WebRuntime>::Value> + AsArray<Runtime = Self::RT> {
    type RT: WebRuntime<Array = Self>;

    fn get(&self, index: usize) -> Result<<Self::RT as WebRuntime>::Value>;

    fn set(&self, index: usize, value: &<Self::RT as WebRuntime>::Value) -> Result<()>;

    fn push(&self, value: <Self::RT as WebRuntime>::Value) -> Result<()>;

    fn pop(&self) -> Result<<Self::RT as WebRuntime>::Value>;

    fn remove(&self, index: usize) -> Result<()>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool;

    fn new(ctx: <Self::RT as WebRuntime>::Context, cap: usize) -> Result<Self>
    where
        Self: Sized;

    fn new_with_data(ctx: <Self::RT as WebRuntime>::Context, data: &[<Self::RT as WebRuntime>::Value]) -> Result<Self>
    where
        Self: Sized;

    fn as_value(&self) -> <Self::RT as WebRuntime>::Value;

    fn as_vec(&self) -> Vec<<Self::RT as WebRuntime>::Value>;
}
