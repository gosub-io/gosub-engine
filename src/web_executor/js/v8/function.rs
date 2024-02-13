use alloc::rc::Rc;
use core::fmt::Display;

use v8::{
    CallbackScope, External, Function, FunctionBuilder, FunctionCallbackArguments,
    FunctionCallbackInfo, Local, ReturnValue,
};

use crate::types::{Error, Result};
use crate::web_executor::js::function::{JSFunctionCallBack, JSFunctionCallBackVariadic};
use crate::web_executor::js::v8::{ctx_from_function_callback_info, V8Context, V8Engine, V8Value};
use crate::web_executor::js::{
    Args, JSError, JSFunction, JSFunctionVariadic, JSRuntime, JSValue, VariadicArgs,
    VariadicArgsInternal,
};

pub struct V8Function<'a> {
    pub(super) ctx: V8Context<'a>,
    pub(super) function: Local<'a, Function>,
}

pub struct V8FunctionCallBack<'a> {
    ctx: V8Context<'a>,
    args: V8Args<'a>,
    ret: Result<Local<'a, v8::Value>>,
}

pub struct V8Args<'a> {
    next: usize,
    args: Vec<Local<'a, v8::Value>>,
}

impl V8Args<'_> {
    fn v8(&self) -> &[Local<v8::Value>] {
        &self.args
    }
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
                context: Rc::clone(&ctx),
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
                context: Rc::clone(&ctx),
                value: *value,
            });
        }

        a
    }
}

impl<'a> JSFunctionCallBack for V8FunctionCallBack<'a> {
    type RT = V8Engine<'a>;
    fn context(&mut self) -> <Self::RT as JSRuntime>::Context {
        Rc::clone(&self.ctx)
    }

    fn args(&mut self) -> &<Self::RT as JSRuntime>::Args {
        &self.args
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn error(&mut self, error: impl Display) {
        let scope = self.ctx.borrow_mut().scope();
        let err = error.to_string();
        let Some(e) = v8::String::new(scope, &err) else {
            eprintln!("failed to create exception string\nexception was: {}", err);
            return;
        };
        scope.throw_exception(Local::from(e));
    }

    fn ret(&mut self, value: <Self::RT as JSRuntime>::Value) {
        self.ret = Ok(value.value);
    }
}

impl<'a> V8Function<'a> {
    pub(crate) fn callback(
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
            ctx: Rc::clone(ctx),
            args,
            ret: Err(Error::JS(JSError::Execution(
                "function was not called".to_owned(),
            ))),
        };

        f(&mut cb);

        match cb.ret {
            Ok(value) => {
                ret.set(value);
            }
            Err(e) => {
                let excep = if let Some(exception) =
                    v8::String::new(ctx.borrow_mut().scope(), &e.to_string())
                {
                    exception.into()
                } else {
                    eprintln!("failed to create exception string\nexception was: {e}");
                    v8::undefined(ctx.borrow_mut().scope()).into()
                };

                ret.set(ctx.borrow_mut().scope().throw_exception(excep));
            }
        }
    }
}

extern "C" fn callback(info: *const FunctionCallbackInfo) {
    let info = unsafe { &*info };
    let scope = &mut unsafe { CallbackScope::new(info) };
    let args = FunctionCallbackArguments::from_function_callback_info(info);
    let rv = ReturnValue::from_function_callback_info(info);
    let external = match <Local<External>>::try_from(args.data()) {
        Ok(external) => external,
        Err(e) => {
            let Some(e) = v8::String::new(scope, &e.to_string()) else {
                eprintln!("failed to create exception string\nexception was: {e}");
                return;
            };
            scope.throw_exception(Local::from(e));
            return;
        }
    };

    let data = unsafe { &mut *(external.value() as *mut CallbackWrapper) };

    let ctx = match ctx_from_function_callback_info(
        unsafe { CallbackScope::new(info) },
        data.ctx.borrow().isolate,
    ) {
        Ok(scope) => scope,
        Err((mut st, e)) => {
            let scope = st.get();
            let Some(e) = v8::String::new(scope, &e.to_string()) else {
                eprintln!("failed to create exception string\nexception was: {e}");
                return;
            };
            scope.throw_exception(Local::from(e));
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
        let ctx = Rc::clone(&ctx);

        let builder: FunctionBuilder<Function> = FunctionBuilder::new_raw(callback);

        let scope = ctx.borrow_mut().scope();

        let data = External::new(scope, CallbackWrapper::new(ctx.clone(), f));

        let function = builder.data(Local::from(data)).build(scope);

        if let Some(function) = function {
            Ok(Self { ctx, function })
        } else {
            Err(Error::JS(JSError::Compile(
                "failed to create function".to_owned(),
            )))
        }
    }

    fn call(&mut self, cb: &mut V8FunctionCallBack) {
        let ret = self.function.call(
            cb.ctx.borrow_mut().scope(),
            Local::from(v8::undefined(cb.ctx.borrow_mut().scope())),
            cb.args.v8(),
        );

        if let Some(value) = ret {
            cb.ret = Ok(value);
        } else {
            cb.ret = Err(Error::JS(JSError::Execution(
                "failed to call a function".to_owned(),
            )));
        };
    }
}

pub struct V8FunctionVariadic<'a> {
    pub(super) ctx: V8Context<'a>,
    pub(super) function: Local<'a, Function>,
}

pub struct V8FunctionCallBackVariadic<'a> {
    ctx: V8Context<'a>,
    args: V8VariadicArgsInternal<'a>,
    ret: Result<Local<'a, v8::Value>>,
}

pub struct V8VariadicArgsInternal<'a> {
    next: usize,
    args: Vec<Local<'a, v8::Value>>,
}

impl V8VariadicArgsInternal<'_> {
    fn v8(&self) -> &[Local<v8::Value>] {
        &self.args
    }
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
                context: Rc::clone(&ctx),
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
                context: Rc::clone(&ctx),
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
            next: 0,
            args: self.as_vec(ctx),
        }
    }
}

pub struct V8VariadicArgs<'a> {
    next: usize,
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
}

impl<'a> JSFunctionCallBackVariadic for V8FunctionCallBackVariadic<'a> {
    type RT = V8Engine<'a>;

    fn context(&mut self) -> <Self::RT as JSRuntime>::Context {
        Rc::clone(&self.ctx)
    }

    fn args(&mut self) -> &<Self::RT as JSRuntime>::VariadicArgsInternal {
        &self.args
    }

    fn len(&self) -> usize {
        self.args.len()
    }

    fn error(&mut self, error: impl Display) {
        let scope = self.ctx.borrow_mut().scope();
        let err = error.to_string();
        let Some(e) = v8::String::new(scope, &err) else {
            eprintln!("failed to create exception string\nexception was: {}", err);
            return;
        };
        scope.throw_exception(Local::from(e));
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
            ctx: Rc::clone(ctx),
            args,
            ret: Err(Error::JS(JSError::Execution(
                "function was not called".to_owned(),
            ))),
        };

        f(&mut cb);

        match cb.ret {
            Ok(value) => {
                ret.set(value);
            }
            Err(e) => {
                let excep = if let Some(exception) =
                    v8::String::new(ctx.borrow_mut().scope(), &e.to_string())
                {
                    exception.into()
                } else {
                    eprintln!("failed to create exception string\nexception was: {e}");
                    v8::undefined(ctx.borrow_mut().scope()).into()
                };

                ret.set(ctx.borrow_mut().scope().throw_exception(excep));
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
            let Some(e) = v8::String::new(&mut scope, &e.to_string()) else {
                eprintln!("failed to create exception string\nexception was: {e}");
                return;
            };
            scope.throw_exception(Local::from(e));
            return;
        }
    };

    let data = unsafe { &mut *(external.value() as *mut CallbackWrapperVariadic) };

    let ctx = match ctx_from_function_callback_info(scope, data.ctx.borrow().isolate) {
        Ok(scope) => scope,
        Err((mut st, e)) => {
            let scope = st.get();
            let Some(e) = v8::String::new(scope, &e.to_string()) else {
                eprintln!("failed to create exception string\nexception was: {e}");
                return;
            };
            scope.throw_exception(Local::from(e));
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
        let ctx = Rc::clone(&ctx);
        let scope = ctx.borrow_mut().scope();

        let builder: FunctionBuilder<Function> = FunctionBuilder::new_raw(callback_variadic);

        let data = External::new(scope, CallbackWrapperVariadic::new(ctx.clone(), f));

        let function = builder.data(Local::from(data)).build(scope);

        if let Some(function) = function {
            Ok(Self { ctx, function })
        } else {
            Err(Error::JS(JSError::Compile(
                "failed to create function".to_owned(),
            )))
        }
    }

    fn call(&mut self, cb: &mut V8FunctionCallBackVariadic) {
        let ret = self.function.call(
            cb.ctx.borrow_mut().scope(),
            Local::from(v8::undefined(cb.ctx.borrow_mut().scope())),
            cb.args.v8(),
        );

        if let Some(value) = ret {
            cb.ret = Ok(value);
        } else {
            cb.ret = Err(Error::JS(JSError::Execution(
                "failed to call a function".to_owned(),
            )));
        };
    }
}
