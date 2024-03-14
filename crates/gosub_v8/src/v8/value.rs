use v8::{Array, Local, Value};

use gosub_shared::types::Result;

use crate::{FromContext, IntoContext, V8Array, V8Context, V8Engine, V8Object};
use gosub_webexecutor::js::{
    ArrayConversion, AsArray, IntoJSValue, JSArray, JSError, JSRuntime, JSType, JSValue, Ref,
};
use gosub_webexecutor::Error;

pub struct V8Value<'a> {
    pub context: V8Context<'a>,
    pub value: Local<'a, Value>,
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

impl<'a> From<V8Array<'a>> for V8Value<'a> {
    fn from(array: V8Array<'a>) -> Self {
        Self {
            context: array.ctx,
            value: array.value.into(),
        }
    }
}

impl<'a> From<V8Object<'a>> for V8Value<'a> {
    fn from(object: V8Object<'a>) -> Self {
        Self {
            context: object.ctx,
            value: object.value.into(),
        }
    }
}

impl<'a> AsArray for V8Value<'a> {
    type Runtime = V8Engine<'a>;

    fn array(&self) -> Result<Ref<<Self::Runtime as JSRuntime>::Array>> {
        Ok(Ref::Owned(self.as_array()?))
    }
}

impl<'a> JSValue for V8Value<'a> {
    type RT = V8Engine<'a>;

    fn as_string(&self) -> Result<String> {
        Ok(self.value.to_rust_string_lossy(self.context.scope()))
    }

    fn as_number(&self) -> Result<f64> {
        if let Some(value) = self.value.number_value(self.context.scope()) {
            Ok(value)
        } else {
            Err(Error::JS(JSError::Conversion(
                "could not convert to number".to_owned(),
            ))
            .into())
        }
    }

    fn as_bool(&self) -> Result<bool> {
        Ok(self.value.boolean_value(self.context.scope()))
    }

    fn as_object(&self) -> Result<<Self::RT as JSRuntime>::Object> {
        if let Some(value) = self.value.to_object(self.context.scope()) {
            Ok(V8Object::from_ctx(V8Context::clone(&self.context), value))
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

    fn as_array(&self) -> Result<<Self::RT as JSRuntime>::Array> {
        let array: Local<Array> = self.value.try_into()?;

        Ok(array.into_ctx(V8Context::clone(&self.context)))
    }

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
            let ctx = self.context.scope();

            let t = self.value.type_of(ctx).to_rust_string_lossy(ctx);

            JSType::Other(t)
        }
    }

    fn new_object(
        ctx: <Self::RT as JSRuntime>::Context,
    ) -> Result<<Self::RT as JSRuntime>::Object> {
        V8Object::new(ctx)
    }

    fn new_array<T: IntoJSValue<Self, Value = Self>>(
        ctx: <Self::RT as JSRuntime>::Context,
        value: &[T],
    ) -> Result<<Self::RT as JSRuntime>::Array> {
        value.to_js_array(ctx)
    }

    fn new_empty_array(
        ctx: <Self::RT as JSRuntime>::Context,
    ) -> Result<<Self::RT as JSRuntime>::Array> {
        V8Array::new(ctx, 0)
    }

    fn new_string(ctx: <Self::RT as JSRuntime>::Context, value: &str) -> Result<Self> {
        if let Some(value) = v8::String::new(ctx.scope(), value) {
            Ok(Self {
                context: V8Context::clone(&ctx),
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
        let value = v8::Number::new(ctx.scope(), value.into());
        Ok(Self {
            context: V8Context::clone(&ctx),
            value: Local::from(value),
        })
    }

    fn new_bool(ctx: <Self::RT as JSRuntime>::Context, value: bool) -> Result<Self> {
        let value = v8::Boolean::new(ctx.scope(), value);
        Ok(Self {
            context: V8Context::clone(&ctx),
            value: Local::from(value),
        })
    }

    fn new_null(ctx: <Self::RT as JSRuntime>::Context) -> Result<Self> {
        let null = v8::null(ctx.scope());

        Ok(Self {
            context: V8Context::clone(&ctx),
            value: Local::from(null),
        })
    }

    fn new_undefined(ctx: <Self::RT as JSRuntime>::Context) -> Result<Self> {
        let scope = ctx.scope();

        let undefined = v8::undefined(scope);

        Ok(Self {
            context: V8Context::clone(&ctx),
            value: Local::from(undefined),
        })
    }
}

#[cfg(test)]
mod tests {
    use gosub_webexecutor::js::{IntoRustValue, JSContext};

    use super::*;

    #[test]
    fn string() {
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
    fn number() {
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
    fn bool() {
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
    fn null() {
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
    fn undefined() {
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
    fn object() {
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
    fn array() {
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
    fn function() {
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
    fn type_of() {
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
    fn new_string() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let value = V8Value::new_string(context, "Hello World!").unwrap();
        assert!(value.is_string());
        assert_eq!(value.as_string().unwrap(), "Hello World!");
    }

    #[test]
    fn new_number() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let value = V8Value::new_number(context, 1234).unwrap();
        assert!(value.is_number());
        assert_eq!(value.as_number().unwrap(), 1234.0);
    }

    #[test]
    fn new_bool() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let value = V8Value::new_bool(context, true).unwrap();
        assert!(value.is_bool());
        assert!(value.as_bool().unwrap());
    }

    #[test]
    fn new_null() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let value = V8Value::new_null(context).unwrap();
        assert!(value.is_null());
    }

    #[test]
    fn new_undefined() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let value = V8Value::new_undefined(context).unwrap();
        assert!(value.is_undefined());
    }

    #[test]
    fn into_rust() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let value = context
            .run(
                r#"
            "Hello World!"
        "#,
            )
            .unwrap();

        let val: String = value.to_rust_value().unwrap();
        assert_eq!(val, "Hello World!");

        let value = context
            .run(
                r#"
            1234
        "#,
            )
            .unwrap();

        let val: u32 = value.to_rust_value().unwrap();
        assert_eq!(val, 1234);
        let val: f64 = value.to_rust_value().unwrap();
        assert_eq!(val, 1234.0);
        let val: u64 = value.to_rust_value().unwrap();
        assert_eq!(val, 1234);

        let value = context
            .run(
                r#"
            true
        "#,
            )
            .unwrap();

        let val: bool = value.to_rust_value().unwrap();
        assert!(val);

        let value = context
            .run(
                r#"
            null
        "#,
            )
            .unwrap();

        let _: () = value.to_rust_value().unwrap();
    }
}
