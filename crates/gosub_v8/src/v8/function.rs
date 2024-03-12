use core::fmt::Display;

use v8::{
    undefined, CallbackScope, External, Function, FunctionBuilder, FunctionCallbackArguments,
    FunctionCallbackInfo, Local, ReturnValue, TryCatch,
};

use gosub_shared::types::Result;
use gosub_webexecutor::js::{
    Args, IntoRustValue, JSError, JSFunction, JSFunctionCallBack, JSFunctionCallBackVariadic,
    JSFunctionVariadic, JSRuntime, VariadicArgs, VariadicArgsInternal,
};
use gosub_webexecutor::Error;

use crate::v8::{ctx_from_function_callback_info, V8Context, V8Engine, V8Value};

pub struct V8Function<'a> {
    pub ctx: V8Context<'a>,
    pub function: Local<'a, Function>,
}

pub struct V8FunctionCallBack<'a> {
    ctx: V8Context<'a>,
    args: V8Args<'a>,
    ret: Result<Local<'a, v8::Value>>,
    is_error: bool,
}

pub struct V8Args<'a> {
    next: usize,
    args: Vec<Local<'a, v8::Value>>,
}

impl<'a> Iterator for V8Args<'a> {
    type Item = Local<'a, v8::Value>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next < self.args.len() {
            let value = *self.args.get(self.next)?;
            self.next += 1;
            Some(value)
        } else {
            None
        }
    }
}

impl<'a> Args for V8Args<'a> {
    type RT = V8Engine<'a>;

    fn get(
        &self,
        index: usize,
        ctx: <Self::RT as JSRuntime>::Context,
    ) -> Option<<Self::RT as JSRuntime>::Value> {
        if index < self.args.len() {
            Some(V8Value {
                context: V8Context::clone(&ctx),
                value: *self.args.get(index)?,
            })
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn as_vec(&self, ctx: <Self::RT as JSRuntime>::Context) -> Vec<<Self::RT as JSRuntime>::Value> {
        let mut a = Vec::with_capacity(self.args.len());
        for i in 0..self.args.len() {
            let Some(value) = self.args.get(i) else {
                continue;
            };

            a.push(V8Value {
                context: V8Context::clone(&ctx),
                value: *value,
            });
        }

        a
    }
}

impl<'a> JSFunctionCallBack for V8FunctionCallBack<'a> {
    type RT = V8Engine<'a>;
    fn context(&mut self) -> <Self::RT as JSRuntime>::Context {
        V8Context::clone(&self.ctx)
    }

    fn args(&mut self) -> &<Self::RT as JSRuntime>::Args {
        &self.args
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn error(&mut self, error: impl Display) {
        self.ctx.error(error);
        self.is_error = true;
    }

    fn ret(&mut self, value: <Self::RT as JSRuntime>::Value) {
        self.ret = Ok(value.value);
    }
}

impl<'a> V8Function<'a> {
    pub fn callback(
        ctx: &V8Context<'a>,
        args: FunctionCallbackArguments<'a>,
        mut ret: ReturnValue,
        f: impl Fn(&mut V8FunctionCallBack<'a>),
    ) {
        let mut a = Vec::with_capacity(args.length() as usize);

        for i in 0..args.length() {
            a.push(args.get(i));
        }

        let args = V8Args { next: 0, args: a };

        let mut cb = V8FunctionCallBack {
            ctx: V8Context::clone(ctx),
            args,
            ret: Err(Error::JS(JSError::Execution("function was not called".to_owned())).into()),
            is_error: false,
        };

        f(&mut cb);

        if cb.is_error {
            ret.set(undefined(ctx.scope()).into());
            return;
        }

        match cb.ret {
            Ok(value) => {
                ret.set(value);
            }
            Err(e) => {
                ctx.error(e);
                ret.set(undefined(ctx.scope()).into())
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

    let ctx = match ctx_from_function_callback_info(scope, data.ctx.borrow().isolate) {
        Ok(scope) => scope,
        Err((mut st, e)) => {
            let scope = st.with_context();
            if let Some(scope) = scope {
                let e = V8Context::create_exception(scope, e);
                if let Some(exception) = e {
                    scope.throw_exception(exception);
                }
            } else {
                let scope = st.get();
                let Some(e) = v8::String::new(scope, &e.to_string()) else {
                    eprintln!("failed to create exception string\nexception was: {e}");
                    return;
                };
                scope.throw_exception(e.into());
            }
            return;
        }
    };

    V8Function::callback(&ctx, args, rv, &data.f);
}

struct CallbackWrapper<'a> {
    ctx: V8Context<'a>,
    f: Box<dyn Fn(&mut V8FunctionCallBack<'a>)>,
}

impl<'a> CallbackWrapper<'a> {
    #[allow(clippy::new_ret_no_self)]
    fn new(
        ctx: V8Context<'a>,
        f: impl Fn(&mut V8FunctionCallBack<'a>) + 'static,
    ) -> *mut std::ffi::c_void {
        let data = Box::new(Self {
            ctx,
            f: Box::new(f),
        });

        Box::into_raw(data) as *mut std::ffi::c_void
    }
}

impl<'a> JSFunction for V8Function<'a> {
    type RT = V8Engine<'a>;
    fn new(
        ctx: <Self::RT as JSRuntime>::Context,
        f: impl Fn(&mut <Self::RT as JSRuntime>::FunctionCallBack) + 'static,
    ) -> Result<Self> {
        let ctx = V8Context::clone(&ctx);

        let builder: FunctionBuilder<Function> = FunctionBuilder::new_raw(callback);

        let scope = ctx.scope();

        let data = External::new(scope, CallbackWrapper::new(ctx.clone(), f));

        let function = builder.data(Local::from(data)).build(scope);

        if let Some(function) = function {
            Ok(Self { ctx, function })
        } else {
            Err(Error::JS(JSError::Compile("failed to create function".to_owned())).into())
        }
    }

    fn call(
        &mut self,
        args: &[<Self::RT as JSRuntime>::Value],
    ) -> Result<<Self::RT as JSRuntime>::Value> {
        let scope = &mut TryCatch::new(self.ctx.scope());

        let recv = Local::from(v8::undefined(scope));
        let ret = self.function.call(
            scope,
            recv,
            args.iter()
                .map(|x| x.value)
                .collect::<Vec<Local<v8::Value>>>()
                .as_slice(),
        );

        if let Some(value) = ret {
            Ok(V8Value {
                context: V8Context::clone(&self.ctx),
                value,
            })
        } else {
            Err(Error::JS(JSError::Execution("failed to call a function".to_owned())).into())
        }
    }
}

pub struct V8FunctionVariadic<'a> {
    pub ctx: V8Context<'a>,
    pub function: Local<'a, Function>,
}

pub struct V8FunctionCallBackVariadic<'a> {
    ctx: V8Context<'a>,
    args: V8VariadicArgsInternal<'a>,
    ret: Result<Local<'a, v8::Value>>,
    is_error: bool,
}

pub struct V8VariadicArgsInternal<'a> {
    next: usize,
    args: Vec<Local<'a, v8::Value>>,
}

impl<'a> Iterator for V8VariadicArgsInternal<'a> {
    type Item = Local<'a, v8::Value>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next < self.args.len() {
            let value = *self.args.get(self.next)?;
            self.next += 1;
            Some(value)
        } else {
            None
        }
    }
}

impl<'a> VariadicArgsInternal for V8VariadicArgsInternal<'a> {
    type RT = V8Engine<'a>;

    fn get(
        &self,
        index: usize,
        ctx: <Self::RT as JSRuntime>::Context,
    ) -> Option<<Self::RT as JSRuntime>::Value> {
        if index < self.args.len() {
            Some(V8Value {
                context: V8Context::clone(&ctx),
                value: *self.args.get(index)?,
            })
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn as_vec(&self, ctx: <Self::RT as JSRuntime>::Context) -> Vec<<Self::RT as JSRuntime>::Value> {
        let mut a = Vec::with_capacity(self.args.len());
        for i in 0..self.args.len() {
            let Some(value) = self.args.get(i) else {
                continue;
            };

            a.push(V8Value {
                context: V8Context::clone(&ctx),
                value: *value,
            });
        }

        a
    }

    fn variadic(
        &self,
        ctx: <Self::RT as JSRuntime>::Context,
    ) -> <Self::RT as JSRuntime>::VariadicArgs {
        V8VariadicArgs {
            args: self.as_vec(ctx),
        }
    }

    fn variadic_start(
        &self,
        start: usize,
        ctx: <Self::RT as JSRuntime>::Context,
    ) -> <Self::RT as JSRuntime>::VariadicArgs {
        V8VariadicArgs {
            args: self.args[start..]
                .iter()
                .map(|x| V8Value {
                    context: V8Context::clone(&ctx),
                    value: *x,
                })
                .collect(),
        }
    }
}

pub struct V8VariadicArgs<'a> {
    args: Vec<V8Value<'a>>,
}

impl<'a> VariadicArgs for V8VariadicArgs<'a> {
    type RT = V8Engine<'a>;

    fn get(&self, index: usize) -> Option<&<Self::RT as JSRuntime>::Value> {
        self.args.get(index)
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn as_vec(&self) -> &Vec<<Self::RT as JSRuntime>::Value> {
        &self.args
    }

    fn as_vec_as<T>(&self) -> Vec<T>
    where
        <Self::RT as JSRuntime>::Value: IntoRustValue<T>,
    {
        self.args
            .iter()
            .map(|x| x.to_rust_value().unwrap())
            .collect()
    }

    fn get_as<T>(&self, index: usize) -> Option<T>
    where
        <Self::RT as JSRuntime>::Value: IntoRustValue<T>,
    {
        self.args.get(index).map(|x| x.to_rust_value().unwrap())
    }
}

impl<'a> JSFunctionCallBackVariadic for V8FunctionCallBackVariadic<'a> {
    type RT = V8Engine<'a>;

    fn context(&mut self) -> <Self::RT as JSRuntime>::Context {
        V8Context::clone(&self.ctx)
    }

    fn args(&mut self) -> &<Self::RT as JSRuntime>::VariadicArgsInternal {
        &self.args
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn error(&mut self, error: impl Display) {
        self.ctx.error(error);
        self.is_error = true;
    }

    fn ret(&mut self, value: <Self::RT as JSRuntime>::Value) {
        self.ret = Ok(value.value);
    }
}

impl<'a> V8FunctionVariadic<'a> {
    fn callback(
        ctx: &V8Context<'a>,
        args: FunctionCallbackArguments<'a>,
        mut ret: ReturnValue,
        f: impl Fn(&mut V8FunctionCallBackVariadic<'a>),
    ) {
        let mut a = Vec::with_capacity(args.length() as usize);

        for i in 0..args.length() {
            a.push(args.get(i));
        }

        let args = V8VariadicArgsInternal { next: 0, args: a };

        let mut cb = V8FunctionCallBackVariadic {
            ctx: V8Context::clone(ctx),
            args,
            ret: Err(Error::JS(JSError::Execution("function was not called".to_owned())).into()),
            is_error: false,
        };

        f(&mut cb);

        if cb.is_error {
            ret.set(undefined(ctx.scope()).into());
            return;
        }

        match cb.ret {
            Ok(value) => {
                ret.set(value);
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

    let ctx = match ctx_from_function_callback_info(scope, data.ctx.borrow().isolate) {
        Ok(scope) => scope,
        Err((mut st, e)) => {
            let scope = st.with_context();
            if let Some(scope) = scope {
                let e = V8Context::create_exception(scope, e);
                if let Some(exception) = e {
                    scope.throw_exception(exception);
                }
            } else {
                let scope = st.get();
                let Some(e) = v8::String::new(scope, &e.to_string()) else {
                    eprintln!("failed to create exception string\nexception was: {e}");
                    return;
                };
                scope.throw_exception(e.into());
            }
            return;
        }
    };

    V8FunctionVariadic::callback(&ctx, args, rv, &data.f);
}

struct CallbackWrapperVariadic<'a> {
    ctx: V8Context<'a>,
    f: Box<dyn Fn(&mut V8FunctionCallBackVariadic<'a>)>,
}

impl<'a> CallbackWrapperVariadic<'a> {
    #[allow(clippy::new_ret_no_self)]
    fn new(
        ctx: V8Context<'a>,
        f: impl Fn(&mut V8FunctionCallBackVariadic<'a>) + 'static,
    ) -> *mut std::ffi::c_void {
        let data = Box::new(Self {
            ctx,
            f: Box::new(f),
        });

        Box::into_raw(data) as *mut _ as *mut std::ffi::c_void
    }
}

impl<'a> JSFunctionVariadic for V8FunctionVariadic<'a> {
    type RT = V8Engine<'a>;
    fn new(
        ctx: <Self::RT as JSRuntime>::Context,
        f: impl Fn(&mut <Self::RT as JSRuntime>::FunctionCallBackVariadic) + 'static,
    ) -> Result<Self> {
        let ctx = V8Context::clone(&ctx);
        let scope = ctx.scope();

        let builder: FunctionBuilder<Function> = FunctionBuilder::new_raw(callback_variadic);

        let data = External::new(scope, CallbackWrapperVariadic::new(ctx.clone(), f));

        let function = builder.data(Local::from(data)).build(scope);

        if let Some(function) = function {
            Ok(Self { ctx, function })
        } else {
            Err(Error::JS(JSError::Compile("failed to create function".to_owned())).into())
        }
    }

    fn call(
        &mut self,
        args: &[<Self::RT as JSRuntime>::Value],
    ) -> Result<<Self::RT as JSRuntime>::Value> {
        let scope = &mut v8::TryCatch::new(self.ctx.scope());
        let recv = Local::from(v8::undefined(scope));
        let ret = self.function.call(
            scope,
            recv,
            args.iter()
                .map(|x| x.value)
                .collect::<Vec<Local<v8::Value>>>()
                .as_slice(),
        );

        if let Some(value) = ret {
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
        Args, IntoJSValue, JSFunction, JSFunctionCallBack, JSFunctionVariadic, JSRuntime, JSValue,
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

                cb.ret(sum.to_js_value(ctx.clone()).unwrap());
            })
            .unwrap()
        };

        let ret = function.call(&[
            1.to_js_value(ctx.clone()).unwrap(),
            2.to_js_value(ctx.clone()).unwrap(),
            3.to_js_value(ctx.clone()).unwrap(),
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

                let val = match sum.to_js_value(ctx.clone()) {
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
            1.to_js_value(ctx.clone()).unwrap(),
            2.to_js_value(ctx.clone()).unwrap(),
            3.to_js_value(ctx.clone()).unwrap(),
            4.to_js_value(ctx.clone()).unwrap(),
        ]);

        assert_eq!(ret.unwrap().as_number().unwrap(), 10.0);
    }
}
