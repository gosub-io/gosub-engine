use alloc::rc::Rc;

use v8::{Local, Value};

use crate::types::Error;
use crate::web_executor::js::v8::{FromContext, V8Context, V8Engine, V8Object};
use crate::web_executor::js::{JSError, JSRuntime, JSType, JSValue, ValueConversion};

pub struct V8Value<'a> {
    pub(crate) context: V8Context<'a>,
    pub(crate) value: Local<'a, Value>,
}

impl<'a> V8Value<'a> {
    pub fn from_value(ctx: V8Context<'a>, value: Local<'a, Value>) -> Self {
        Self {
            context: ctx,
            value,
        }
    }
}

macro_rules! impl_is {
    ($name:ident) => {
        fn $name(&self) -> bool {
            self.value.$name()
        }
    };
}

impl<'a> JSValue for V8Value<'a> {
    type Context = V8Context<'a>;
    type Object = V8Object<'a>;

    fn as_string(&self) -> crate::types::Result<String> {
        Ok(self
            .value
            .to_rust_string_lossy(self.context.borrow_mut().scope()))
    }

    fn as_number(&self) -> crate::types::Result<f64> {
        if let Some(value) = self.value.number_value(self.context.borrow_mut().scope()) {
            Ok(value)
        } else {
            Err(Error::JS(JSError::Conversion(
                "could not convert to number".to_owned(),
            )))
        }
    }

    fn as_bool(&self) -> crate::types::Result<bool> {
        Ok(self.value.boolean_value(self.context.borrow_mut().scope()))
    }

    fn as_object(&self) -> crate::types::Result<Self::Object> {
        if let Some(value) = self.value.to_object(self.context.borrow_mut().scope()) {
            Ok(V8Object::from_ctx(Rc::clone(&self.context), value))
        } else {
            Err(Error::JS(JSError::Conversion(
                "could not convert to number".to_owned(),
            )))
        }
    }

    impl_is!(is_string);
    impl_is!(is_number);
    impl_is!(is_object);
    impl_is!(is_array);
    impl_is!(is_null);
    impl_is!(is_undefined);
    impl_is!(is_function);

    fn is_bool(&self) -> bool {
        self.value.is_boolean()
    }

    fn type_of(&self) -> JSType {
        //There is a v8::Value::type_of() method, but it returns a string, which is not what we want.
        if self.is_string() {
            JSType::String
        } else if self.is_number() {
            JSType::Number
        } else if self.is_bool() {
            JSType::Boolean
        } else if self.is_object() {
            JSType::Object
        } else if self.is_array() {
            JSType::Array
        } else if self.is_null() {
            JSType::Null
        } else if self.is_undefined() {
            JSType::Undefined
        } else if self.is_function() {
            JSType::Function
        } else {
            let ctx = self.context.borrow_mut().scope();

            let t = self.value.type_of(ctx).to_rust_string_lossy(ctx);

            JSType::Other(t)
        }
    }

    fn new_string(ctx: Self::Context, value: &str) -> crate::types::Result<Self> {
        if let Some(value) = v8::String::new(ctx.borrow_mut().scope(), value) {
            Ok(Self {
                context: Rc::clone(&ctx),
                value: Local::from(value),
            })
        } else {
            Err(Error::JS(JSError::Conversion(
                "could not convert to string".to_owned(),
            )))
        }
    }

    fn new_number<N: Into<f64>>(ctx: Self::Context, value: N) -> crate::types::Result<Self> {
        let value = v8::Number::new(ctx.borrow_mut().scope(), value.into());
        Ok(Self {
            context: Rc::clone(&ctx),
            value: Local::from(value),
        })
    }

    fn new_bool(ctx: Self::Context, value: bool) -> crate::types::Result<Self> {
        let value = v8::Boolean::new(ctx.borrow_mut().scope(), value);
        Ok(Self {
            context: Rc::clone(&ctx),
            value: Local::from(value),
        })
    }

    fn new_null(ctx: Self::Context) -> crate::types::Result<Self> {
        let null = v8::null(ctx.borrow_mut().scope());

        Ok(Self {
            context: Rc::clone(&ctx),
            value: Local::from(null),
        })
    }

    fn new_undefined(ctx: Self::Context) -> crate::types::Result<Self> {
        let undefined = v8::undefined(ctx.borrow_mut().scope());

        Ok(Self {
            context: Rc::clone(&ctx),
            value: Local::from(undefined),
        })
    }
}
