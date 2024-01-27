use crate::types::Result;
use crate::web_executor::js::{JSContext, JSError, JSObject, JSRuntime, JSValue};

struct Function<T: JSFunction>(pub T);

//trait for JS functions (interop between JS and Rust)
pub trait JSFunction {
    type CB: JSFunctionCallBack;
    type Context: JSContext;
    type Value: JSValue;

    fn new(ctx: Self::Context, func: impl Fn(&mut Self::CB) + 'static) -> Result<Self>
    where
        Self: Sized;

    fn call(&mut self, callback: &mut Self::CB);
}

pub trait JSFunctionCallBack {
    type Args: Args;
    type Context: JSContext;
    type Value: JSValue;

    fn context(&mut self) -> Self::Context;

    fn args(&mut self) -> &Self::Args;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn ret(&mut self, value: Self::Value);
}

pub trait Args: Iterator {
    type Context: JSContext;
    type Value: JSValue;

    fn get(&self, index: usize, ctx: Self::Context) -> Option<Self::Value>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn as_vec(&self, ctx: Self::Context) -> Vec<Self::Value>;
}

//extra trait for variadic functions to mark them as such
pub trait JSFunctionVariadic {
    type CB: JSFunctionCallBackVariadic;
    type Context: JSContext;
    type Value: JSValue;

    fn new(ctx: Self::Context, func: impl Fn(&mut Self::CB) + 'static) -> Result<Self>
    where
        Self: Sized;

    fn call(&mut self, callback: &mut Self::CB);
}

pub trait JSFunctionCallBackVariadic {
    type Args: VariadicArgsInternal;
    type Context: JSContext;
    type Value: JSValue;

    fn context(&mut self) -> Self::Context;

    fn args(&mut self) -> &Self::Args;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn ret(&mut self, value: Self::Value);
}

pub trait VariadicArgsInternal: Iterator {
    type Context: JSContext;
    type Value: JSValue;

    type Args: VariadicArgs;

    fn get(&self, index: usize, ctx: Self::Context) -> Option<Self::Value>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn as_vec(&self, ctx: Self::Context) -> Vec<Self::Value>;

    fn variadic(&self, ctx: Self::Context) -> Self::Args;
}

pub trait VariadicArgs {
    type Value: JSValue;

    fn get(&self, index: usize) -> Option<&Self::Value>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn as_vec(&self) -> &Vec<Self::Value>;
}
