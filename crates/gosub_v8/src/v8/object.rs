use core::fmt::Display;
use std::ffi::c_void;

use v8::{
    AccessorConfiguration, External, HandleScope, Local, Name, Object, PropertyCallbackArguments,
    ReturnValue, Value,
};

use gosub_shared::types::Result;
use gosub_webexecutor::js::{
    JSError, JSGetterCallback, JSObject, JSRuntime, JSSetterCallback, JSValue,
};
use gosub_webexecutor::Error;

use crate::{
    ctx_from, FromContext, V8Context, V8Ctx, V8Engine, V8Function, V8FunctionVariadic, V8Value,
};

pub struct V8Object<'a> {
    pub ctx: V8Context<'a>,
    pub value: Local<'a, Object>,
}

pub struct GetterCallback<'a> {
    ctx: V8Context<'a>,
    ret: V8Value<'a>,
}

impl V8Object<'_> {
    pub fn new(ctx: V8Context) -> Result<V8Object> {
        let scope = ctx.scope();
        let value = Object::new(scope);
        Ok(V8Object { ctx, value })
    }
}

impl<'a> JSGetterCallback for GetterCallback<'a> {
    type RT = V8Engine<'a>;

    fn context(&mut self) -> &mut <Self::RT as JSRuntime>::Context {
        &mut self.ctx
    }

    fn error(&mut self, error: impl Display) {
        self.ctx.error(error);
    }

    fn ret(&mut self, value: <Self::RT as JSRuntime>::Value) {
        self.ret = value;
    }
}

pub struct SetterCallback<'a> {
    ctx: V8Context<'a>,
    value: V8Value<'a>,
}

impl<'a> JSSetterCallback for SetterCallback<'a> {
    type RT = V8Engine<'a>;

    fn context(&mut self) -> &mut <Self::RT as JSRuntime>::Context {
        &mut self.ctx
    }

    fn error(&mut self, error: impl Display) {
        self.ctx.error(error)
    }

    fn value(&mut self) -> &<Self::RT as JSRuntime>::Value {
        &self.value
    }
}

struct GetterSetter<'a> {
    ctx: V8Context<'a>,
    getter: Box<dyn Fn(&mut GetterCallback<'a>)>,
    setter: Box<dyn Fn(&mut SetterCallback<'a>)>,
}

impl<'a> JSObject for V8Object<'a> {
    type RT = V8Engine<'a>;

    fn set_property(&self, name: &str, value: &V8Value) -> Result<()> {
        let Some(name) = v8::String::new(self.ctx.scope(), name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        if self
            .value
            .set(self.ctx.scope(), name.into(), value.value)
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
        let Some(name) = v8::String::new(self.ctx.scope(), name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        let scope = self.ctx.scope();

        self.value
            .get(scope, name.into())
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

        let try_catch = &mut v8::TryCatch::new(self.ctx.scope());

        let Some(ret) = function
            .call(try_catch, self.value.into(), &args)
            .map(|v| V8Value::from_value(self.ctx.clone(), v))
        else {
            return Err(V8Ctx::report_exception(try_catch).into());
        };

        Ok(ret)
    }

    fn set_method(&self, name: &str, func: &V8Function) -> Result<()> {
        let Some(name) = v8::String::new(self.ctx.scope(), name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        if !func.function.is_function() {
            return Err(
                Error::JS(JSError::Generic("property is not a function".to_owned())).into(),
            );
        }

        if self
            .value
            .set(self.ctx.scope(), name.into(), func.function.into())
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
        let Some(name) = v8::String::new(self.ctx.scope(), name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        if !func.function.is_function() {
            return Err(
                Error::JS(JSError::Generic("property is not a function".to_owned())).into(),
            );
        }

        if self
            .value
            .set(self.ctx.scope(), name.into(), func.function.into())
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
        let name = v8::String::new(self.ctx.scope(), name)
            .ok_or_else(|| Error::JS(JSError::Generic("failed to create a string".to_owned())))?;

        let scope = self.ctx.scope();

        let gs = Box::new(GetterSetter {
            ctx: self.ctx.clone(),
            getter,
            setter,
        });

        let data = External::new(scope, Box::into_raw(gs) as *mut c_void);

        let config = AccessorConfiguration::new(
            |scope: &mut HandleScope,
             _name: Local<Name>,
             args: PropertyCallbackArguments,
             mut rv: ReturnValue| {
                let external = match Local::<External>::try_from(args.data()) {
                    Ok(external) => external,
                    Err(e) => {
                        let Some(e) = V8Context::create_exception(scope, e) else {
                            eprintln!("failed to create exception string\nexception was: {e}");
                            return;
                        };
                        scope.throw_exception(e);
                        return;
                    }
                };

                let gs = unsafe { &*(external.value() as *const GetterSetter) };

                let isolate = gs.ctx.borrow().isolate;

                let ctx = match ctx_from(scope, isolate) {
                    Ok(ctx) => ctx,
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

                let ret = match V8Value::new_undefined(ctx.clone()) {
                    Ok(ret) => ret,
                    Err(e) => {
                        ctx.error(e);
                        return;
                    }
                };

                let mut gc = GetterCallback { ctx, ret };

                (gs.getter)(&mut gc);

                rv.set(gc.ret.value);
            },
        )
        .setter(
            |scope: &mut HandleScope,
             _name: Local<Name>,
             value: Local<Value>,
             args: PropertyCallbackArguments,
             _rv: ReturnValue<()>| {
                let external = match Local::<External>::try_from(args.data()) {
                    Ok(external) => external,
                    Err(e) => {
                        let Some(e) = V8Context::create_exception(scope, e) else {
                            eprintln!("failed to create exception string\nexception was: {e}");
                            return;
                        };
                        scope.throw_exception(e);
                        return;
                    }
                };

                let gs = unsafe { &*(external.value() as *const GetterSetter) };

                let ctx = match ctx_from(scope, gs.ctx.borrow().isolate) {
                    Ok(ctx) => ctx,
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

                let val = V8Value::from_value(ctx.clone(), value);

                let mut sc = SetterCallback { ctx, value: val };

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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::to_string;

    use gosub_webexecutor::js::{
        IntoJSValue, JSFunction, JSFunctionCallBack, JSFunctionCallBackVariadic,
        JSFunctionVariadic, VariadicArgsInternal,
    };

    use crate::v8::{V8FunctionCallBack, V8FunctionCallBackVariadic};

    use super::*;

    #[test]
    fn test_object() {
        let mut engine = V8Engine::new();
        let ctx = engine.new_context().unwrap();

        let obj = V8Object::new(ctx.clone()).unwrap();

        let value = V8Value::new_string(ctx.clone(), "value").unwrap();
        obj.set_property("key", &value).unwrap();

        let value = obj.get_property("key").unwrap();
        assert_eq!(value.as_string().unwrap(), "value");
    }

    #[test]
    fn test_object_accessor() {
        let mut engine = V8Engine::new();
        let ctx = engine.new_context().unwrap();

        let string = Rc::new(RefCell::new("value".to_string()));

        let getter = {
            let string = Rc::clone(&string);
            Box::new(move |cb: &mut GetterCallback| {
                let value = string.borrow().to_js_value(cb.context().clone()).unwrap();
                cb.ret(value);
            })
        };

        let setter = {
            let string = Rc::clone(&string);
            Box::new(move |cb: &mut SetterCallback| {
                let value = cb.value().as_string().unwrap();
                *string.borrow_mut() = value;
            })
        };

        let obj = V8Object::new(ctx.clone()).unwrap();
        obj.set_property_accessor("key", getter, setter).unwrap();

        let value = obj.get_property("key").unwrap();
        assert_eq!(value.as_string().unwrap(), "value");
        //TODO modify value and test
    }

    #[test]
    fn test_object_method() {
        let mut engine = V8Engine::new();
        let ctx = engine.new_context().unwrap();

        let obj = V8Object::new(ctx.clone()).unwrap();

        let _called = Rc::new(RefCell::new(false));
        let mut func = V8Function::new(ctx.clone(), |cb: &mut V8FunctionCallBack| {
            let value = V8Value::new_string(cb.context().clone(), "value").unwrap();
            cb.ret(value);
        })
        .unwrap();

        func.call(&[]).unwrap();

        obj.set_method("key", &func).unwrap();

        let value = obj.call_method("key", &[]).unwrap();
        assert_eq!(value.as_string().unwrap(), "value");
    }

    #[test]
    fn test_object_method_variadic() {
        let mut engine = V8Engine::new();
        let ctx = engine.new_context().unwrap();

        let obj = V8Object::new(ctx.clone()).unwrap();

        let _called = Rc::new(RefCell::new(false));
        let func = V8FunctionVariadic::new(ctx.clone(), |cb: &mut V8FunctionCallBackVariadic| {
            let ctx = cb.context().clone();

            let args_str = cb
                .args()
                .as_vec(ctx.clone())
                .iter()
                .map(|v| v.as_string().unwrap())
                .collect::<Vec<_>>();

            let value = V8Value::new_string(ctx, &to_string(&args_str).unwrap()).unwrap();

            cb.ret(value);
        })
        .unwrap();

        obj.set_method_variadic("key", &func).unwrap();

        let value = obj
            .call_method(
                "key",
                &[
                    &V8Value::new_string(ctx.clone(), "value1").unwrap(),
                    &V8Value::new_string(ctx.clone(), "value2").unwrap(),
                    &V8Value::new_undefined(ctx.clone()).unwrap(),
                    &V8Value::new_null(ctx.clone()).unwrap(),
                    &V8Value::new_number(ctx.clone(), 42).unwrap(),
                ],
            )
            .unwrap();
        assert_eq!(
            value.as_string().unwrap(),
            "[\"value1\",\"value2\",\"undefined\",\"null\",\"42\"]"
        );
    }
}
