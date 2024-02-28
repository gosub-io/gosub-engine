use core::fmt::Display;

use gosub_shared::types::Result;

use crate::js::{JSContext, JSObject, JSRuntime, JSValue};

struct Function<T: JSFunction>(pub T);

//trait for JS functions (interop between JS and Rust)
pub trait JSFunction {
    type RT: JSRuntime;

    fn new(
        ctx: <Self::RT as JSRuntime>::Context,
        func: impl Fn(&mut <Self::RT as JSRuntime>::FunctionCallBack) + 'static,
    ) -> Result<Self>
    where
        Self: Sized;

    fn call(
        &mut self,
        args: &[<Self::RT as JSRuntime>::Value],
    ) -> Result<<Self::RT as JSRuntime>::Value>;
}

pub trait JSFunctionCallBack {
    type RT: JSRuntime;

    fn context(&mut self) -> <Self::RT as JSRuntime>::Context;

    fn args(&mut self) -> &<Self::RT as JSRuntime>::Args;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn error(&mut self, error: impl Display);

    fn ret(&mut self, value: <Self::RT as JSRuntime>::Value);
}

pub trait Args: Iterator {
    type RT: JSRuntime;

    fn get(
        &self,
        index: usize,
        ctx: <Self::RT as JSRuntime>::Context,
    ) -> Option<<Self::RT as JSRuntime>::Value>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn as_vec(&self, ctx: <Self::RT as JSRuntime>::Context) -> Vec<<Self::RT as JSRuntime>::Value>;
}

//extra trait for variadic functions to mark them as such
pub trait JSFunctionVariadic {
    type RT: JSRuntime;
    fn new(
        ctx: <Self::RT as JSRuntime>::Context,
        func: impl Fn(&mut <Self::RT as JSRuntime>::FunctionCallBackVariadic) + 'static,
    ) -> Result<Self>
    where
        Self: Sized;

    fn call(
        &mut self,
        args: &[<Self::RT as JSRuntime>::Value],
    ) -> Result<<Self::RT as JSRuntime>::Value>;
}

pub trait JSFunctionCallBackVariadic {
    type RT: JSRuntime;

    fn context(&mut self) -> <Self::RT as JSRuntime>::Context;

    fn args(&mut self) -> &<Self::RT as JSRuntime>::VariadicArgsInternal;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn error(&mut self, error: impl Display);

    fn ret(&mut self, value: <Self::RT as JSRuntime>::Value);
}

pub trait VariadicArgsInternal: Iterator {
    type RT: JSRuntime;

    fn get(
        &self,
        index: usize,
        ctx: <Self::RT as JSRuntime>::Context,
    ) -> Option<<Self::RT as JSRuntime>::Value>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn as_vec(&self, ctx: <Self::RT as JSRuntime>::Context) -> Vec<<Self::RT as JSRuntime>::Value>;

    fn variadic(
        &self,
        ctx: <Self::RT as JSRuntime>::Context,
    ) -> <Self::RT as JSRuntime>::VariadicArgs;
}

pub trait VariadicArgs {
    type RT: JSRuntime;

    fn get(&self, index: usize) -> Option<&<Self::RT as JSRuntime>::Value>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn as_vec(&self) -> &Vec<<Self::RT as JSRuntime>::Value>;
}
