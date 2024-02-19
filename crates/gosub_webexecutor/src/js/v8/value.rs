use std::rc::Rc;

use v8::{Local, Value};

use crate::js::v8::{FromContext, V8Context, V8Engine, V8Object};
use crate::js::{JSError, JSRuntime, JSType, JSValue, ValueConversion};
use crate::Error;
use gosub_shared::types::Result;

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
    type RT = V8Engine<'a>;

    fn as_string(&self) -> Result<String> {
        Ok(self
            .value
            .to_rust_string_lossy(self.context.borrow_mut().scope()))
    }

    fn as_number(&self) -> Result<f64> {
        if let Some(value) = self.value.number_value(self.context.borrow_mut().scope()) {
            Ok(value)
        } else {
            Err(Error::JS(JSError::Conversion(
                "could not convert to number".to_owned(),
            ))
            .into())
        }
    }

    fn as_bool(&self) -> Result<bool> {
        Ok(self.value.boolean_value(self.context.borrow_mut().scope()))
    }

    fn as_object(&self) -> Result<<Self::RT as JSRuntime>::Object> {
        if let Some(value) = self.value.to_object(self.context.borrow_mut().scope()) {
            Ok(V8Object::from_ctx(Rc::clone(&self.context), value))
        } else {
            Err(Error::JS(JSError::Conversion(
                "could not convert to number".to_owned(),
            ))
            .into())
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
        } else if self.is_array() {
            JSType::Array
        } else if self.is_null() {
            JSType::Null
        } else if self.is_undefined() {
            JSType::Undefined
        } else if self.is_function() {
            JSType::Function
        } else if self.is_object() {
            JSType::Object
        } else {
            let ctx = self.context.borrow_mut().scope();

            let t = self.value.type_of(ctx).to_rust_string_lossy(ctx);

            JSType::Other(t)
        }
    }

    fn new_string(ctx: <Self::RT as JSRuntime>::Context, value: &str) -> Result<Self> {
        if let Some(value) = v8::String::new(ctx.borrow_mut().scope(), value) {
            Ok(Self {
                context: Rc::clone(&ctx),
                value: Local::from(value),
            })
        } else {
            Err(Error::JS(JSError::Conversion(
                "could not convert to string".to_owned(),
            ))
            .into())
        }
    }

    fn new_number<N: Into<f64>>(ctx: <Self::RT as JSRuntime>::Context, value: N) -> Result<Self> {
        let value = v8::Number::new(ctx.borrow_mut().scope(), value.into());
        Ok(Self {
            context: Rc::clone(&ctx),
            value: Local::from(value),
        })
    }

    fn new_bool(ctx: <Self::RT as JSRuntime>::Context, value: bool) -> Result<Self> {
        let value = v8::Boolean::new(ctx.borrow_mut().scope(), value);
        Ok(Self {
            context: Rc::clone(&ctx),
            value: Local::from(value),
        })
    }

    fn new_null(ctx: <Self::RT as JSRuntime>::Context) -> Result<Self> {
        let null = v8::null(ctx.borrow_mut().scope());

        Ok(Self {
            context: Rc::clone(&ctx),
            value: Local::from(null),
        })
    }

    fn new_undefined(ctx: <Self::RT as JSRuntime>::Context) -> Result<Self> {
        let undefined = v8::undefined(ctx.borrow_mut().scope());

        Ok(Self {
            context: Rc::clone(&ctx),
            value: Local::from(undefined),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::web_executor::js::JSContext;

    use super::*;

    #[test]
    fn test_v8_value_string() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = context
            .run(
                r#"
            "Hello World!"
        "#,
            )
            .unwrap();

        assert!(value.is_string());
        assert_eq!(value.as_string().unwrap(), "Hello World!");
    }

    #[test]
    fn test_v8_value_number() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = context
            .run(
                r#"
            1234
        "#,
            )
            .unwrap();

        assert!(value.is_number());
        assert_eq!(value.as_number().unwrap(), 1234.0);
    }

    #[test]
    fn test_v8_value_bool() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = context
            .run(
                r#"
            true
        "#,
            )
            .unwrap();

        assert!(value.is_bool());
        assert!(value.as_bool().unwrap());
    }

    #[test]
    fn test_v8_value_null() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = context
            .run(
                r#"
            null
        "#,
            )
            .unwrap();

        assert!(value.is_null());
    }

    #[test]
    fn test_v8_value_undefined() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = context
            .run(
                r#"
            undefined
        "#,
            )
            .unwrap();

        assert!(value.is_undefined());
    }

    #[test]
    fn test_v8_value_object() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = context
            .run(
                r#"
            obj = { "hello": "world" }
            obj
        "#,
            )
            .unwrap();

        assert!(value.is_object());
    }

    #[test]
    fn test_v8_value_array() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = context
            .run(
                r#"
            [1, 2, 3]
        "#,
            )
            .unwrap();

        assert!(value.is_array());
        // assert_eq!(value.as_array().unwrap(), vec![1.0, 2.0, 3.0]); //TODO
        assert_eq!(value.as_string().unwrap(), "1,2,3");
    }

    #[test]
    fn test_v8_value_function() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = context
            .run(
                r#"
            function hello() {
                return "world";
            }
            hello
        "#,
            )
            .unwrap();

        assert!(value.is_function());
    }

    #[test]
    fn test_v8_value_type_of() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        // Test String
        {
            let value = context
                .run(
                    r#"
            "Hello World!"
            "#,
                )
                .unwrap();
            assert_eq!(value.type_of(), JSType::String);
        }

        // Test Number
        {
            let value = context
                .run(
                    r#"
            1234
            "#,
                )
                .unwrap();
            assert_eq!(value.type_of(), JSType::Number);
        }

        // Test Boolean
        {
            let value = context
                .run(
                    r#"
            true
            "#,
                )
                .unwrap();
            assert_eq!(value.type_of(), JSType::Boolean);
        }

        // Test Object
        {
            let value = context
                .run(
                    r#"
            obj = {"key": "value"}
            obj
            "#,
                )
                .unwrap();
            assert_eq!(value.type_of(), JSType::Object);
        }

        // Test Array
        {
            let value = context
                .run(
                    r#"
            [1, 2, 3]
            "#,
                )
                .unwrap();
            assert_eq!(value.type_of(), JSType::Array);
        }

        // Test Null
        {
            let value = context
                .run(
                    r#"
            null
            "#,
                )
                .unwrap();
            assert_eq!(value.type_of(), JSType::Null);
        }

        // Test Undefined
        {
            let value = context
                .run(
                    r#"
            undefined
            "#,
                )
                .unwrap();
            assert_eq!(value.type_of(), JSType::Undefined);
        }

        // Test Function
        {
            let value = context
                .run(
                    r#"
            function test() {}

            test
            "#,
                )
                .unwrap();
            assert_eq!(value.type_of(), JSType::Function);
        }
    }

    #[test]
    fn test_v8_value_new_string() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = V8Value::new_string(context, "Hello World!").unwrap();
        assert!(value.is_string());
        assert_eq!(value.as_string().unwrap(), "Hello World!");
    }

    #[test]
    fn test_v8_value_new_number() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = V8Value::new_number(context, 1234).unwrap();
        assert!(value.is_number());
        assert_eq!(value.as_number().unwrap(), 1234.0);
    }

    #[test]
    fn test_v8_value_new_bool() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = V8Value::new_bool(context, true).unwrap();
        assert!(value.is_bool());
        assert!(value.as_bool().unwrap());
    }

    #[test]
    fn test_v8_value_new_null() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = V8Value::new_null(context).unwrap();
        assert!(value.is_null());
    }

    #[test]
    fn test_v8_value_new_undefined() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = V8Value::new_undefined(context).unwrap();
        assert!(value.is_undefined());
    }
}
