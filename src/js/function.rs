use crate::js::{JSContext, JSError, JSObject, JSValue};

struct Function<T: JSFunction>(pub T);

//trait for JS functions (interopt between JS and Rust)
pub(super) trait JSFunction {
    type Context: JSContext;
    type CB: JSFunctionCallBack;

    fn call(&mut self, callback: &mut Self::CB);
}

pub(super) trait JSFunctionCallBack {
    type Context: JSContext;

    type Value: JSValue;

    fn context(&mut self) -> Self::Context;

    fn args(&mut self) -> Vec<Self::Value>;

    fn ret(&mut self, value: Self::Value);
}

pub(super) trait VariadicArgs: Iterator {
    type Value: JSValue;

    fn get(&self, index: usize) -> Option<Self::Value>;

    fn len(&self) -> usize;

    fn as_vec(&self) -> Vec<Self::Value>;
}

pub(super) trait Args: Iterator {
    type Value: JSValue;

    fn get(&self, index: usize) -> Option<Self::Value>;

    fn len(&self) -> usize;

    fn as_vec(&self) -> Vec<Self::Value>;
}

pub(super) struct VariadicFunction<T: JSFunctionVariadic>(pub T);

//extra trait for variadic functions to mark them as such
pub(super) trait JSFunctionVariadic {
    type Context: JSContext;

    type CB: JSFunctionCallBackVariadic;

    fn call(&mut self, callback: &mut Self::CB);
}

pub(super) trait JSFunctionCallBackVariadic {
    type Context: JSContext;

    type Value: JSValue;

    type Args: VariadicArgs;

    fn scope(&mut self) -> Self::Context;

    fn args(&mut self) -> &Self::Args;

    fn ret(&mut self, value: Self::Value);

    fn error(&mut self, error: JSError);
}
