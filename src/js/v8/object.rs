use v8::{Local, Object};

use crate::js::v8::{FromContext, V8Context, V8Ctx, V8Value};
use crate::js::{JSArray, JSError, JSObject, JSValue};
use crate::types::{Error, Result};

pub struct V8Object<'a> {
    ctx: V8Context<'a>,
    pub(crate) value: Local<'a, Object>,
}

impl<'a> JSObject for V8Object<'a> {
    type Value = V8Value<'a>;

    fn set_property(&self, name: &str, value: &Self::Value) -> Result<()> {
        let Some(name) = v8::String::new(self.ctx.borrow_mut().scope(), name) else {
            return Err(Error::JS(JSError::Generic(
                "failed to create a string".to_owned(),
            )));
        };

        if self
            .value
            .set(self.ctx.borrow_mut().scope(), name.into(), value.value)
            .is_none()
        {
            Err(Error::JS(JSError::Generic(
                "failed to set a property in an object".to_owned(),
            )))
        } else {
            Ok(())
        }
    }

    fn get_property(&self, name: &str) -> Result<Self::Value> {
        let Some(name) = v8::String::new(self.ctx.borrow_mut().scope(), name) else {
            return Err(Error::JS(JSError::Generic(
                "failed to create a string".to_owned(),
            )));
        };

        self.value
            .get(self.ctx.borrow_mut().scope(), name.into())
            .ok_or_else(|| {
                Error::JS(JSError::Generic(
                    "failed to get a property from an object".to_owned(),
                ))
            })
            .map(|value| V8Value::from_value(self.ctx.clone(), value))
    }

    fn call_method(&self, name: &str, args: &[&Self::Value]) -> Result<Self::Value> {
        let func = self.get_property(name)?.value;

        if !func.is_function() {
            return Err(Error::JS(JSError::Generic(
                "property is not a function".to_owned(),
            )));
        }

        let function = Local::<v8::Function>::try_from(func).unwrap();

        let args: Vec<Local<v8::Value>> = args.iter().map(|v| v.value).collect();

        let try_catch = &mut v8::TryCatch::new(self.ctx.borrow_mut().scope());

        let Some(ret) = function
            .call(try_catch, self.value.into(), &args)
            .map(|v| V8Value::from_value(self.ctx.clone(), v))
        else {
            return Err(V8Ctx::report_exception(try_catch));
        };

        Ok(ret)
    }
}

impl<'a> FromContext<'a, Local<'a, Object>> for V8Object<'a> {
    fn from_ctx(ctx: V8Context<'a>, object: Local<'a, Object>) -> Self {
        Self { ctx, value: object }
    }
}
