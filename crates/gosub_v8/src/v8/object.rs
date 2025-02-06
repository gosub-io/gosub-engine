use core::fmt::Display;
use std::ffi::c_void;

use gosub_shared::types::Result;
use gosub_webexecutor::js::{JSError, WebGetterCallback, WebObject, WebRuntime, WebSetterCallback, WebValue};
use gosub_webexecutor::Error;
use v8::{
    AccessorConfiguration, External, Global, HandleScope, Local, Name, Object, PropertyCallbackArguments, ReturnValue,
    Value,
};

use crate::{FromContext, V8Context, V8Ctx, V8Engine, V8Function, V8FunctionVariadic, V8Value};

#[derive(Clone)]
pub struct V8Object {
    pub ctx: V8Context,
    pub value: Global<Object>,
}

pub struct GetterCallback {
    ctx: V8Context,
    ret: V8Value,
}

impl V8Object {
    pub fn new(ctx: V8Context) -> Result<V8Object> {
        let mut scope = ctx.scope();
        let value = Object::new(&mut scope);

        let value = Global::new(&mut scope, value);

        drop(scope);

        Ok(V8Object { ctx, value })
    }
}

impl WebGetterCallback for GetterCallback {
    type RT = V8Engine;

    fn context(&mut self) -> &mut <Self::RT as WebRuntime>::Context {
        &mut self.ctx
    }

    fn error(&mut self, error: impl Display) {
        self.ctx.error(error);
    }

    fn ret(&mut self, value: <Self::RT as WebRuntime>::Value) {
        self.ret = value;
    }
}

pub struct SetterCallback {
    ctx: V8Context,
    value: V8Value,
}

impl WebSetterCallback for SetterCallback {
    type RT = V8Engine;

    fn context(&mut self) -> &mut <Self::RT as WebRuntime>::Context {
        &mut self.ctx
    }

    fn error(&mut self, error: impl Display) {
        self.ctx.error(error)
    }

    fn value(&mut self) -> &<Self::RT as WebRuntime>::Value {
        &self.value
    }
}

struct GetterSetter {
    ctx: V8Context,
    getter: Box<dyn Fn(&mut GetterCallback)>,
    setter: Box<dyn Fn(&mut SetterCallback)>,
}

impl WebObject for V8Object {
    type RT = V8Engine;

    fn set_property(&self, name: &str, value: &V8Value) -> Result<()> {
        let scope = &mut self.ctx.scope();

        let Some(name) = v8::String::new(scope, name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        let obj = self.value.open(scope);
        let value = Local::new(scope, value.value.clone());

        if obj.set(scope, name.into(), value).is_none() {
            Err(Error::JS(JSError::Generic("failed to set a property in an object".to_owned())).into())
        } else {
            Ok(())
        }
    }

    fn get_property(&self, name: &str) -> Result<<Self::RT as WebRuntime>::Value> {
        let scope = &mut self.ctx.scope();

        let Some(name) = v8::String::new(scope, name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        let obj = self.value.open(scope);

        obj.get(scope, name.into())
            .ok_or_else(|| Error::JS(JSError::Generic("failed to get a property from an object".to_owned())).into())
            .map(|value| {
                let value = Global::new(scope, value);
                V8Value::from_value(self.ctx.clone(), value)
            })
    }

    fn call_method(
        &self,
        name: &str,
        args: &[&<Self::RT as WebRuntime>::Value],
    ) -> Result<<Self::RT as WebRuntime>::Value> {
        let scope = &mut self.ctx.scope();

        let Some(name) = v8::String::new(scope, name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        let obj = self.value.open(scope);

        let func = obj.get(scope, name.into()).ok_or_else(|| {
            anyhow::Error::new(Error::JS(JSError::Generic(
                "failed to get a property from an object".to_owned(),
            )))
        })?;

        if !func.is_function() {
            return Err(Error::JS(JSError::Generic("property is not a function".to_owned())).into());
        }

        let function = Local::<v8::Function>::try_from(func)?;

        let args: Vec<Local<Value>> = args.iter().map(|v| Local::new(scope, &v.value)).collect();

        let try_catch = &mut v8::TryCatch::new(scope);

        let recv = Local::new(try_catch, self.value.clone());

        let Some(ret) = function
            .call(try_catch, recv.into(), &args)
            .map(|v| V8Value::from_value(self.ctx.clone(), Global::new(try_catch, v)))
        else {
            return Err(V8Ctx::report_exception(try_catch).into());
        };

        Ok(ret)
    }

    fn set_method(&self, name: &str, func: &V8Function) -> Result<()> {
        let scope = &mut self.ctx.scope();

        let Some(name) = v8::String::new(scope, name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        let f = Local::new(scope, func.function.clone());

        let obj = self.value.open(scope);

        if obj.set(scope, name.into(), f.into()).is_none() {
            Err(Error::JS(JSError::Generic("failed to set a property in an object".to_owned())).into())
        } else {
            Ok(())
        }
    }

    fn set_method_variadic(&self, name: &str, func: &V8FunctionVariadic) -> Result<()> {
        let scope = &mut self.ctx.scope();

        let Some(name) = v8::String::new(scope, name) else {
            return Err(Error::JS(JSError::Generic("failed to create a string".to_owned())).into());
        };

        let f = Local::new(scope, func.function.clone());

        let obj = self.value.open(scope);

        if obj.set(scope, name.into(), f.into()).is_none() {
            Err(Error::JS(JSError::Generic("failed to set a property in an object".to_owned())).into())
        } else {
            Ok(())
        }
    }

    fn set_property_accessor(
        &self,
        name: &str,
        getter: Box<dyn Fn(&mut <Self::RT as WebRuntime>::GetterCB)>,
        setter: Box<dyn Fn(&mut <Self::RT as WebRuntime>::SetterCB)>,
    ) -> Result<()> {
        let scope = &mut self.ctx.scope();
        let name = v8::String::new(scope, name)
            .ok_or_else(|| Error::JS(JSError::Generic("failed to create a string".to_owned())))?;

        let gs = Box::new(GetterSetter {
            ctx: self.ctx.clone(),
            getter,
            setter,
        });

        let data = External::new(scope, Box::into_raw(gs) as *mut c_void);

        let config = AccessorConfiguration::new(
            |scope: &mut HandleScope, _name: Local<Name>, args: PropertyCallbackArguments, mut rv: ReturnValue| {
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

                let ret = match V8Value::new_undefined(gs.ctx.clone()) {
                    Ok(ret) => ret,
                    Err(e) => {
                        gs.ctx.error(e);
                        return;
                    }
                };

                let sg = gs.ctx.set_parent_scope(HandleScope::new(scope));

                let mut gc = GetterCallback {
                    ctx: gs.ctx.clone(),
                    ret,
                };
                //TODO: do we need to drop the scope here?
                (gs.getter)(&mut gc);

                drop(sg);

                rv.set(Local::new(scope, gc.ret.value));
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

                let val = V8Value::from_value(gs.ctx.clone(), Global::new(scope, value));

                let sg = gs.ctx.set_parent_scope(HandleScope::new(scope));

                let mut sc = SetterCallback {
                    ctx: gs.ctx.clone(),
                    value: val,
                };

                (gs.setter)(&mut sc);

                drop(sg);
            },
        )
        .data(Local::from(data));

        let obj = self.value.open(scope);

        obj.set_accessor_with_configuration(scope, name.into(), config);

        Ok(())
    }

    fn new(ctx: &<Self::RT as WebRuntime>::Context) -> Result<Self> {
        Self::new(ctx.clone())
    }
}

impl FromContext<Local<'_, Object>> for V8Object {
    fn from_ctx(ctx: V8Context, object: Local<'_, Object>) -> Self {
        let object = Global::new(&mut ctx.isolate(), object);

        Self { ctx, value: object }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::to_string;

    use gosub_webexecutor::js::{
        IntoWebValue, VariadicArgsInternal, WebFunction, WebFunctionCallBack, WebFunctionCallBackVariadic,
        WebFunctionVariadic,
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
                let value = string.borrow().to_web_value(cb.context().clone()).unwrap();
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
