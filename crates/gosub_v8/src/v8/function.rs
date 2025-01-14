use core::fmt::Display;
use std::ops::DerefMut;
use v8::{
    undefined, CallbackScope, External, Function, FunctionBuilder, FunctionCallbackArguments, FunctionCallbackInfo,
    Global, HandleScope, Local, ReturnValue, TryCatch,
};

use gosub_shared::types::Result;
use gosub_webexecutor::js::{
    Args, IntoRustValue, JSError, VariadicArgs, VariadicArgsInternal, WebFunction, WebFunctionCallBack,
    WebFunctionCallBackVariadic, WebFunctionVariadic, WebRuntime,
};
use gosub_webexecutor::Error;

use crate::v8::{V8Context, V8Engine, V8Value};

pub struct V8Function {
    pub ctx: V8Context,
    pub function: Global<Function>,
}

pub struct V8FunctionCallBack {
    ctx: V8Context,
    args: V8Args,
    ret: Result<Global<v8::Value>>,
    is_error: bool,
}

pub struct V8Args {
    next: usize,
    args: Vec<Global<v8::Value>>,
}

impl Iterator for V8Args {
    type Item = Global<v8::Value>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next < self.args.len() {
            let value = self.args.get(self.next)?;
            self.next += 1;
            Some(value.clone())
        } else {
            None
        }
    }
}

impl Args for V8Args {
    type RT = V8Engine;

    fn get(&self, index: usize, ctx: <Self::RT as WebRuntime>::Context) -> Option<<Self::RT as WebRuntime>::Value> {
        if index < self.args.len() {
            Some(V8Value {
                context: V8Context::clone(&ctx),
                value: self.args.get(index)?.clone(),
            })
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn as_vec(&self, ctx: <Self::RT as WebRuntime>::Context) -> Vec<<Self::RT as WebRuntime>::Value> {
        let mut a = Vec::with_capacity(self.args.len());
        for i in 0..self.args.len() {
            let Some(value) = self.args.get(i) else {
                continue;
            };

            a.push(V8Value {
                context: V8Context::clone(&ctx),
                value: value.clone(),
            });
        }

        a
    }
}

impl WebFunctionCallBack for V8FunctionCallBack {
    type RT = V8Engine;
    fn context(&mut self) -> <Self::RT as WebRuntime>::Context {
        V8Context::clone(&self.ctx)
    }

    fn args(&mut self) -> &<Self::RT as WebRuntime>::Args {
        &self.args
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn error(&mut self, error: impl Display) {
        self.ctx.error(error);
        self.is_error = true;
    }

    fn ret(&mut self, value: <Self::RT as WebRuntime>::Value) {
        self.ret = Ok(value.value);
    }
}

impl V8Function {
    pub fn callback(
        ctx: &V8Context,
        args: FunctionCallbackArguments,
        mut ret: ReturnValue,
        f: impl Fn(&mut V8FunctionCallBack),
    ) {
        let mut a = Vec::with_capacity(args.length() as usize);

        let mut isolate = ctx.isolate();

        for i in 0..args.length() {
            a.push(Global::new(isolate.deref_mut(), args.get(i)));
        }

        let args = V8Args { next: 0, args: a };

        let mut cb = V8FunctionCallBack {
            ctx: V8Context::clone(ctx),
            args,
            ret: Err(Error::JS(JSError::Execution("function was not called".to_owned())).into()),
            is_error: false,
        };

        f(&mut cb);

        let mut scope = cb.ctx.scope();

        if cb.is_error {
            ret.set(undefined(&mut scope).into());
            return;
        }

        match cb.ret {
            Ok(value) => {
                let value = Local::new(&mut scope, value);

                ret.set(value);
            }
            Err(e) => {
                ret.set(undefined(&mut scope).into());
                ctx.error(e);
            }
        }
    }
}

extern "C" fn callback(info: *const FunctionCallbackInfo) {
    let info = unsafe { &*info };
    let args = FunctionCallbackArguments::from_function_callback_info(info);
    let mut scope = unsafe { CallbackScope::new(info) };
    let rv = ReturnValue::from_function_callback_info(info);
    let external = match <Local<External>>::try_from(args.data()) {
        Ok(external) => external,
        Err(e) => {
            let excep = V8Context::create_exception(&mut scope, e.to_string());
            if let Some(exception) = excep {
                scope.throw_exception(exception);
            }
            return;
        }
    };

    let data = unsafe { &mut *(external.value() as *mut CallbackWrapper) };

    let sg = data.ctx.set_parent_scope(HandleScope::new(&mut scope));

    V8Function::callback(&data.ctx, args, rv, &data.f);

    drop(sg);
}

struct CallbackWrapper {
    ctx: V8Context,
    f: Box<dyn Fn(&mut V8FunctionCallBack)>,
}

impl CallbackWrapper {
    #[allow(clippy::new_ret_no_self)]
    fn new(ctx: V8Context, f: impl Fn(&mut V8FunctionCallBack) + 'static) -> *mut std::ffi::c_void {
        let data = Box::new(Self { ctx, f: Box::new(f) });

        Box::into_raw(data) as *mut std::ffi::c_void
    }
}

impl WebFunction for V8Function {
    type RT = V8Engine;
    fn new(
        ctx: <Self::RT as WebRuntime>::Context,
        f: impl Fn(&mut <Self::RT as WebRuntime>::FunctionCallBack) + 'static,
    ) -> Result<Self> {
        let ctx = V8Context::clone(&ctx);

        let builder: FunctionBuilder<Function> = FunctionBuilder::new_raw(callback);

        let mut scope = ctx.scope();

        let data = External::new(&mut scope, CallbackWrapper::new(ctx.clone(), f));

        let function = builder.data(Local::from(data)).build(&mut scope);

        if let Some(function) = function {
            let function = Global::new(&mut scope, function);
            drop(scope);
            Ok(Self { ctx, function })
        } else {
            Err(Error::JS(JSError::Compile("failed to create function".to_owned())).into())
        }
    }

    fn call(&mut self, args: &[<Self::RT as WebRuntime>::Value]) -> Result<<Self::RT as WebRuntime>::Value> {
        let scope = &mut self.ctx.scope();
        let scope = &mut TryCatch::new(scope);

        let recv = Local::from(v8::undefined(scope));

        let function = self.function.open(scope);

        let args = args
            .iter()
            .map(|x| Local::new(scope, x.value.clone()))
            .collect::<Vec<Local<v8::Value>>>();

        let ret = function.call(scope, recv, &args);

        if let Some(value) = ret {
            let value = Global::new(scope, value);

            Ok(V8Value {
                context: V8Context::clone(&self.ctx),
                value,
            })
        } else {
            Err(Error::JS(JSError::Execution("failed to call a function".to_owned())).into())
        }
    }
}

pub struct V8FunctionVariadic {
    pub ctx: V8Context,
    pub function: Global<Function>,
}

pub struct V8FunctionCallBackVariadic {
    ctx: V8Context,
    args: V8VariadicArgsInternal,
    ret: Result<Global<v8::Value>>,
    is_error: bool,
}

pub struct V8VariadicArgsInternal {
    next: usize,
    args: Vec<Global<v8::Value>>,
}

impl Iterator for V8VariadicArgsInternal {
    type Item = Global<v8::Value>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next < self.args.len() {
            let value = self.args.get(self.next)?;
            self.next += 1;
            Some(value.clone())
        } else {
            None
        }
    }
}

impl VariadicArgsInternal for V8VariadicArgsInternal {
    type RT = V8Engine;

    fn get(&self, index: usize, ctx: <Self::RT as WebRuntime>::Context) -> Option<<Self::RT as WebRuntime>::Value> {
        if index < self.args.len() {
            Some(V8Value {
                context: V8Context::clone(&ctx),
                value: self.args.get(index)?.clone(),
            })
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn as_vec(&self, ctx: <Self::RT as WebRuntime>::Context) -> Vec<<Self::RT as WebRuntime>::Value> {
        let mut a = Vec::with_capacity(self.args.len());
        for i in 0..self.args.len() {
            let Some(value) = self.args.get(i) else {
                continue;
            };

            a.push(V8Value {
                context: V8Context::clone(&ctx),
                value: value.clone(),
            });
        }

        a
    }

    fn variadic(&self, ctx: <Self::RT as WebRuntime>::Context) -> <Self::RT as WebRuntime>::VariadicArgs {
        V8VariadicArgs { args: self.as_vec(ctx) }
    }

    fn variadic_start(
        &self,
        start: usize,
        ctx: <Self::RT as WebRuntime>::Context,
    ) -> <Self::RT as WebRuntime>::VariadicArgs {
        V8VariadicArgs {
            args: self.args[start..]
                .iter()
                .map(|x| V8Value {
                    context: V8Context::clone(&ctx),
                    value: x.clone(),
                })
                .collect(),
        }
    }
}

pub struct V8VariadicArgs {
    args: Vec<V8Value>,
}

impl VariadicArgs for V8VariadicArgs {
    type RT = V8Engine;

    fn get(&self, index: usize) -> Option<&<Self::RT as WebRuntime>::Value> {
        self.args.get(index)
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn as_vec(&self) -> &Vec<<Self::RT as WebRuntime>::Value> {
        &self.args
    }

    fn as_vec_as<T>(&self) -> Vec<T>
    where
        <Self::RT as WebRuntime>::Value: IntoRustValue<T>,
    {
        self.args.iter().map(|x| x.to_rust_value().unwrap()).collect()
    }

    fn get_as<T>(&self, index: usize) -> Option<T>
    where
        <Self::RT as WebRuntime>::Value: IntoRustValue<T>,
    {
        self.args.get(index).map(|x| x.to_rust_value().unwrap())
    }
}

impl WebFunctionCallBackVariadic for V8FunctionCallBackVariadic {
    type RT = V8Engine;

    fn context(&mut self) -> <Self::RT as WebRuntime>::Context {
        V8Context::clone(&self.ctx)
    }

    fn args(&mut self) -> &<Self::RT as WebRuntime>::VariadicArgsInternal {
        &self.args
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn error(&mut self, error: impl Display) {
        self.ctx.error(error);
        self.is_error = true;
    }

    fn ret(&mut self, value: <Self::RT as WebRuntime>::Value) {
        self.ret = Ok(value.value);
    }
}

impl V8FunctionVariadic {
    fn callback(
        ctx: &V8Context,
        args: FunctionCallbackArguments,
        mut ret: ReturnValue,
        f: impl Fn(&mut V8FunctionCallBackVariadic),
    ) {
        let mut c = ctx.borrow_mut();

        let mut a = Vec::with_capacity(args.length() as usize);

        for i in 0..args.length() {
            a.push(Global::new(c.isolate(), args.get(i)));
        }

        let args = V8VariadicArgsInternal { next: 0, args: a };

        let mut cb = V8FunctionCallBackVariadic {
            ctx: V8Context::clone(ctx),
            args,
            ret: Err(Error::JS(JSError::Execution("function was not called".to_owned())).into()),
            is_error: false,
        };

        drop(c);

        f(&mut cb);

        let scope = &mut ctx.scope();

        if cb.is_error {
            ret.set(undefined(scope).into());
            return;
        }

        match cb.ret {
            Ok(value) => {
                ret.set(Local::new(scope, value));
            }
            Err(e) => {
                ctx.error(e);
            }
        }
    }
}

extern "C" fn callback_variadic(info: *const FunctionCallbackInfo) {
    let info = unsafe { &*info };
    let mut scope = unsafe { CallbackScope::new(info) };
    let args = FunctionCallbackArguments::from_function_callback_info(info);
    let rv = ReturnValue::from_function_callback_info(info);
    let external = match <Local<External>>::try_from(args.data()) {
        Ok(external) => external,
        Err(e) => {
            let Some(e) = V8Context::create_exception(&mut scope, e) else {
                eprintln!("failed to create exception string\nexception was: {e}");
                return;
            };

            scope.throw_exception(e);
            return;
        }
    };

    let data = unsafe { &mut *(external.value() as *mut CallbackWrapperVariadic) };

    let sg = data.ctx.set_parent_scope(HandleScope::new(&mut scope));

    V8FunctionVariadic::callback(&data.ctx, args, rv, &data.f);

    drop(sg);
}

struct CallbackWrapperVariadic {
    ctx: V8Context,
    f: Box<dyn Fn(&mut V8FunctionCallBackVariadic)>,
}

impl CallbackWrapperVariadic {
    #[allow(clippy::new_ret_no_self)]
    fn new(ctx: V8Context, f: impl Fn(&mut V8FunctionCallBackVariadic) + 'static) -> *mut std::ffi::c_void {
        let data = Box::new(Self { ctx, f: Box::new(f) });

        Box::into_raw(data) as *mut _ as *mut std::ffi::c_void
    }
}

impl WebFunctionVariadic for V8FunctionVariadic {
    type RT = V8Engine;
    fn new(
        ctx: <Self::RT as WebRuntime>::Context,
        f: impl Fn(&mut <Self::RT as WebRuntime>::FunctionCallBackVariadic) + 'static,
    ) -> Result<Self> {
        let mut scope = ctx.scope();

        let builder: FunctionBuilder<Function> = FunctionBuilder::new_raw(callback_variadic);

        let data = External::new(&mut scope, CallbackWrapperVariadic::new(ctx.clone(), f));

        let function = builder.data(Local::from(data)).build(&mut scope);

        if let Some(function) = function {
            let function = Global::new(&mut scope, function);
            drop(scope);
            Ok(Self { ctx, function })
        } else {
            Err(Error::JS(JSError::Compile("failed to create function".to_owned())).into())
        }
    }

    fn call(&mut self, args: &[<Self::RT as WebRuntime>::Value]) -> Result<<Self::RT as WebRuntime>::Value> {
        let scope = &mut self.ctx.scope();

        let scope = &mut v8::TryCatch::new(scope);
        let recv = Local::from(v8::undefined(scope));

        let function = self.function.open(scope);
        let args = args
            .iter()
            .map(|x| Local::new(scope, x.value.clone()))
            .collect::<Vec<Local<v8::Value>>>();

        let ret = function.call(scope, recv, &args);

        if let Some(value) = ret {
            let value = Global::new(scope, value);

            Ok(V8Value {
                context: V8Context::clone(&self.ctx),
                value,
            })
        } else {
            Err(Error::JS(JSError::Execution("failed to call a function".to_owned())).into())
        }
    }
}

#[cfg(test)]
mod tests {
    use gosub_webexecutor::js::{
        Args, IntoWebValue, WebFunction, WebFunctionCallBack, WebFunctionVariadic, WebRuntime, WebValue,
    };

    use crate::v8::{V8Engine, V8Function, V8FunctionVariadic};

    use super::*;

    #[test]
    fn function_test() {
        let ctx = V8Engine::new().new_context().unwrap();

        let mut function = {
            let ctx = ctx.clone();
            V8Function::new(ctx.clone(), move |cb| {
                let ctx = cb.context();
                assert_eq!(cb.len(), 3);

                let sum = cb
                    .args()
                    .as_vec(ctx.clone())
                    .iter()
                    .fold(0, |acc, x| acc + x.as_number().unwrap() as i32);

                cb.ret(sum.to_web_value(ctx.clone()).unwrap());
            })
            .unwrap()
        };

        let ret = function.call(&[
            1.to_web_value(ctx.clone()).unwrap(),
            2.to_web_value(ctx.clone()).unwrap(),
            3.to_web_value(ctx.clone()).unwrap(),
        ]);

        assert_eq!(ret.unwrap().as_number().unwrap(), 6.0);
    }

    #[test]
    fn function_variadic_test() {
        let ctx = V8Engine::new().new_context().unwrap();

        let mut function = {
            let ctx = ctx.clone();
            V8FunctionVariadic::new(ctx.clone(), move |cb| {
                let ctx = cb.context();
                let sum = cb.args().as_vec(ctx.clone()).iter().fold(0, |acc, x| {
                    acc + match x.as_number() {
                        Ok(x) => x as i32,
                        Err(e) => {
                            cb.error(e);
                            return 0;
                        }
                    }
                });

                let val = match sum.to_web_value(ctx.clone()) {
                    Ok(val) => val,
                    Err(e) => {
                        cb.error(e);
                        return;
                    }
                };
                cb.ret(val);
            })
            .unwrap()
        };

        let ret = function.call(&[
            1.to_web_value(ctx.clone()).unwrap(),
            2.to_web_value(ctx.clone()).unwrap(),
            3.to_web_value(ctx.clone()).unwrap(),
            4.to_web_value(ctx.clone()).unwrap(),
        ]);

        assert_eq!(ret.unwrap().as_number().unwrap(), 10.0);
    }
}
