use core::fmt::Display;

use gosub_shared::types::Result;

use crate::js::IntoRustValue;
use crate::js::WebRuntime;

//trait for JS functions (interop between JS and Rust)
pub trait WebFunction {
    type RT: WebRuntime<Function = Self>;

    fn new(
        ctx: <Self::RT as WebRuntime>::Context,
        func: impl Fn(&mut <Self::RT as WebRuntime>::FunctionCallBack) + 'static,
    ) -> Result<Self>
    where
        Self: Sized;

    fn call(&mut self, args: &[<Self::RT as WebRuntime>::Value]) -> Result<<Self::RT as WebRuntime>::Value>;
}

pub trait WebFunctionCallBack {
    type RT: WebRuntime<FunctionCallBack = Self>;

    fn context(&mut self) -> <Self::RT as WebRuntime>::Context;

    fn args(&mut self) -> &<Self::RT as WebRuntime>::Args;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn error(&mut self, error: impl Display);

    fn ret(&mut self, value: <Self::RT as WebRuntime>::Value);
}

pub trait Args: Iterator {
    type RT: WebRuntime<Args = Self>;

    fn get(&self, index: usize, ctx: <Self::RT as WebRuntime>::Context) -> Option<<Self::RT as WebRuntime>::Value>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn as_vec(&self, ctx: <Self::RT as WebRuntime>::Context) -> Vec<<Self::RT as WebRuntime>::Value>;
}

//extra trait for variadic functions to mark them as such
pub trait WebFunctionVariadic {
    type RT: WebRuntime<FunctionVariadic = Self>;
    fn new(
        ctx: <Self::RT as WebRuntime>::Context,
        func: impl Fn(&mut <Self::RT as WebRuntime>::FunctionCallBackVariadic) + 'static,
    ) -> Result<Self>
    where
        Self: Sized;

    fn call(&mut self, args: &[<Self::RT as WebRuntime>::Value]) -> Result<<Self::RT as WebRuntime>::Value>;
}

pub trait WebFunctionCallBackVariadic {
    type RT: WebRuntime<FunctionCallBackVariadic = Self>;

    fn context(&mut self) -> <Self::RT as WebRuntime>::Context;

    fn args(&mut self) -> &<Self::RT as WebRuntime>::VariadicArgsInternal;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn error(&mut self, error: impl Display);

    fn ret(&mut self, value: <Self::RT as WebRuntime>::Value);
}

pub trait VariadicArgsInternal: Iterator {
    type RT: WebRuntime<VariadicArgsInternal = Self>;

    fn get(&self, index: usize, ctx: <Self::RT as WebRuntime>::Context) -> Option<<Self::RT as WebRuntime>::Value>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn as_vec(&self, ctx: <Self::RT as WebRuntime>::Context) -> Vec<<Self::RT as WebRuntime>::Value>;

    fn variadic(&self, ctx: <Self::RT as WebRuntime>::Context) -> <Self::RT as WebRuntime>::VariadicArgs;

    fn variadic_start(
        &self,
        start: usize,
        ctx: <Self::RT as WebRuntime>::Context,
    ) -> <Self::RT as WebRuntime>::VariadicArgs;
}

pub trait VariadicArgs {
    type RT: WebRuntime<VariadicArgs = Self>;

    fn get(&self, index: usize) -> Option<&<Self::RT as WebRuntime>::Value>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn as_vec(&self) -> &Vec<<Self::RT as WebRuntime>::Value>;

    fn as_vec_as<T>(&self) -> Vec<T>
    where
        <Self::RT as WebRuntime>::Value: IntoRustValue<T>;

    fn get_as<T>(&self, index: usize) -> Option<T>
    where
        <Self::RT as WebRuntime>::Value: IntoRustValue<T>;
}
