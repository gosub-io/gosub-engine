use core::fmt::Display;
use std::ffi::c_void;

use v8::{
    AccessorConfiguration, External, HandleScope, Local, Name, Object, PropertyCallbackArguments,
    ReturnValue, Value,
};

use crate::js::v8::{
    ctx_from, FromContext, V8Context, V8Ctx, V8Engine, V8Function, V8FunctionCallBack,
    V8FunctionVariadic, V8Value,
};
use crate::js::{
    JSArray, JSError, JSGetterCallback, JSObject, JSRuntime, JSSetterCallback, JSValue,
};
use crate::Error;
use gosub_shared::types::Result;

pub struct V8Object<'a> {
    ctx: V8Context<'a>,
    pub(crate) value: Local<'a, Object>,
}

pub struct GetterCallback<'a, 'r> {
    ctx: V8Context<'a>,
    ret: &'r mut V8Value<'a>,
}

impl<'a> JSGetterCallback for GetterCallback<'a, '_> {
    type RT = V8Engine<'a>;

    fn context(&mut self) -> &mut <Self::RT as JSRuntime>::Context {
        &mut self.ctx
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
        *self.ret = value;
    }
}

pub struct SetterCallback<'a, 'v> {
    ctx: V8Context<'a>,
    value: &'v V8Value<'a>,
}

impl<'a, 'v> JSSetterCallback for SetterCallback<'a, 'v> {
    type RT = V8Engine<'a>;

    fn context(&mut self) -> &mut <Self::RT as JSRuntime>::Context {
        &mut self.ctx
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

    fn value(&mut self) -> &'v <Self::RT as JSRuntime>::Value {
        self.value
    }
}

struct GetterSetter<'a, 'r> {
    ctx: V8Context<'a>,
    getter: Box<dyn Fn(&mut GetterCallback<'a, 'r>)>,
    setter: Box<dyn Fn(&mut SetterCallback<'a, 'r>)>,
}

impl<'a> JSObject for V8Object<'a> {
    type RT = V8Engine<'a>;

    fn set_property(&self, name: &str, value: &V8Value) -> Result<()> {
        let Some(name) = v8::String::new(self.ctx.borrow_mut().scope(), name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        if self
            .value
            .set(self.ctx.borrow_mut().scope(), name.into(), value.value)
            .is_none()
        {
            Err(Error::JS(JSError::Generic(
                "failed to set a property in an object".to_owned(),
            ))
            .into())
        } else {
            Ok(())
        }
    }

    fn get_property(&self, name: &str) -> Result<<Self::RT as JSRuntime>::Value> {
        let Some(name) = v8::String::new(self.ctx.borrow_mut().scope(), name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        self.value
            .get(self.ctx.borrow_mut().scope(), name.into())
            .ok_or_else(|| {
                Error::JS(JSError::Generic(
                    "failed to get a property from an object".to_owned(),
                ))
                .into()
            })
            .map(|value| V8Value::from_value(self.ctx.clone(), value))
    }

    fn call_method(
        &self,
        name: &str,
        args: &[&<Self::RT as JSRuntime>::Value],
    ) -> Result<<Self::RT as JSRuntime>::Value> {
        let func = self.get_property(name)?.value;

        if !func.is_function() {
            return Err(
                Error::JS(JSError::Generic("property is not a function".to_owned())).into(),
            );
        }

        let function = Local::<v8::Function>::try_from(func).unwrap();

        let args: Vec<Local<Value>> = args.iter().map(|v| v.value).collect();

        let try_catch = &mut v8::TryCatch::new(self.ctx.borrow_mut().scope());

        let Some(ret) = function
            .call(try_catch, self.value.into(), &args)
            .map(|v| V8Value::from_value(self.ctx.clone(), v))
        else {
            return Err(V8Ctx::report_exception(try_catch).into());
        };

        Ok(ret)
    }

    fn set_method(&self, name: &str, func: &V8Function) -> Result<()> {
        let Some(name) = v8::String::new(self.ctx.borrow_mut().scope(), name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        if !func.function.is_function() {
            return Err(
                Error::JS(JSError::Generic("property is not a function".to_owned())).into(),
            );
        }

        if self
            .value
            .set(
                self.ctx.borrow_mut().scope(),
                name.into(),
                func.function.into(),
            )
            .is_none()
        {
            Err(Error::JS(JSError::Generic(
                "failed to set a property in an object".to_owned(),
            ))
            .into())
        } else {
            Ok(())
        }
    }

    fn set_method_variadic(&self, name: &str, func: &V8FunctionVariadic) -> Result<()> {
        let Some(name) = v8::String::new(self.ctx.borrow_mut().scope(), name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        if !func.function.is_function() {
            return Err(
                Error::JS(JSError::Generic("property is not a function".to_owned())).into(),
            );
        }

        if self
            .value
            .set(
                self.ctx.borrow_mut().scope(),
                name.into(),
                func.function.into(),
            )
            .is_none()
        {
            Err(Error::JS(JSError::Generic(
                "failed to set a property in an object".to_owned(),
            ))
            .into())
        } else {
            Ok(())
        }
    }

    fn set_property_accessor(
        &self,
        name: &str,
        getter: Box<dyn Fn(&mut <Self::RT as JSRuntime>::GetterCB)>,
        setter: Box<dyn Fn(&mut <Self::RT as JSRuntime>::SetterCB)>,
    ) -> Result<()> {
        let name = v8::String::new(self.ctx.borrow_mut().scope(), name)
            .ok_or_else(|| Error::JS(JSError::Generic("failed to create a string".to_owned())))?;

        let scope = self.ctx.borrow_mut().scope();

        let gs = Box::new(GetterSetter {
            ctx: self.ctx.clone(),
            getter,
            setter,
        });

        let data = External::new(scope, Box::into_raw(gs) as *mut c_void);

        let config = AccessorConfiguration::new(
            |scope: &mut HandleScope,
             name: Local<Name>,
             args: PropertyCallbackArguments,
             mut rv: ReturnValue| {
                let external = match Local::<External>::try_from(args.data()) {
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

                let gs = unsafe { &*(external.value() as *const GetterSetter) };

                let ctx = scope.get_current_context();

                let ctx = match ctx_from(scope, gs.ctx.borrow().isolate) {
                    Ok(ctx) => ctx,
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

                let mut ret = match V8Value::new_undefined(ctx.clone()) {
                    Ok(ret) => ret,
                    Err(e) => {
                        let scope = ctx.borrow_mut().scope();
                        let Some(e) = v8::String::new(scope, &e.to_string()) else {
                            eprintln!("failed to create exception string\nexception was: {e}");
                            return;
                        };
                        scope.throw_exception(Local::from(e));
                        return;
                    }
                };

                let mut gc = GetterCallback { ctx, ret: &mut ret };

                (gs.getter)(&mut gc);

                rv.set(ret.value);
            },
        )
        .setter(
            |scope: &mut HandleScope,
             name: Local<Name>,
             value: Local<Value>,
             args: PropertyCallbackArguments,
             rv: ReturnValue| {
                let external = match Local::<External>::try_from(args.data()) {
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

                let gs = unsafe { &*(external.value() as *const GetterSetter) };

                let mut ctx = scope.get_current_context();

                let ctx = match ctx_from(scope, gs.ctx.borrow().isolate) {
                    Ok(ctx) => ctx,
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

                let mut val = V8Value::from_value(ctx.clone(), value);

                let mut sc = SetterCallback {
                    ctx,
                    value: &mut val,
                };

                (gs.setter)(&mut sc);
            },
        )
        .data(Local::from(data));

        self.value
            .set_accessor_with_configuration(scope, name.into(), config);

        Ok(())
    }
}

impl<'a> FromContext<'a, Local<'a, Object>> for V8Object<'a> {
    fn from_ctx(ctx: V8Context<'a>, object: Local<'a, Object>) -> Self {
        Self { ctx, value: object }
    }
}
