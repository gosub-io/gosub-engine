use std::ops::DerefMut;
use v8::{Array, Global, Local, Value};

use gosub_shared::types::Result;

use crate::{FromContext, IntoContext, V8Array, V8Context, V8Engine, V8Object};
use gosub_webexecutor::js::{
    ArrayConversion, AsArray, IntoWebValue, JSError, JSType, Ref, WebArray, WebRuntime, WebValue,
};
use gosub_webexecutor::Error;

pub struct V8Value {
    pub context: V8Context,
    pub value: Global<Value>,
}

impl V8Value {
    pub fn from_value(ctx: V8Context, value: Global<Value>) -> Self {
        Self { context: ctx, value }
    }

    pub fn from_local(ctx: V8Context, value: Local<Value>) -> Self {
        Self {
            value: Global::new(&mut ctx.isolate(), value),
            context: ctx,
        }
    }

    #[allow(unused)]
    fn from_local_iso(isolate: &mut v8::Isolate, ctx: V8Context, value: Local<Value>) -> Self {
        Self {
            context: ctx,
            value: Global::new(isolate, value),
        }
    }
}

macro_rules! impl_is {
    ($name:ident) => {
        fn $name(&self) -> bool {
            let mut iso = self.context.isolate();

            let value = self.value.open(&mut iso);

            value.$name()
        }
    };
}

impl From<V8Array> for V8Value {
    fn from(array: V8Array) -> Self {
        let mut scope = array.ctx.scope();

        let value: Local<Value> = Local::new(&mut scope, array.value).into();

        let value = Global::new(&mut scope, value);

        drop(scope);

        Self {
            value,
            context: array.ctx,
        }
    }
}

impl From<V8Object> for V8Value {
    fn from(object: V8Object) -> Self {
        let mut scope = object.ctx.scope();

        let value: Local<Value> = Local::new(&mut scope, object.value).into();

        let value = Global::new(&mut scope, value);

        drop(scope);

        Self {
            value,
            context: object.ctx,
        }
    }
}

impl AsArray for V8Value {
    type Runtime = V8Engine;

    fn array(&self) -> Result<Ref<'_, <Self::Runtime as WebRuntime>::Array>> {
        Ok(Ref::Owned(self.as_array()?))
    }
}

impl WebValue for V8Value {
    type RT = V8Engine;

    fn as_string(&self) -> Result<String> {
        let mut scope = self.context.scope();

        let value = self.value.open(&mut scope);

        Ok(value.to_rust_string_lossy(&mut scope))
    }

    fn as_number(&self) -> Result<f64> {
        let mut scope = self.context.scope();

        let value = self.value.open(&mut scope);
        if let Some(value) = value.number_value(&mut scope) {
            Ok(value)
        } else {
            Err(Error::JS(JSError::Conversion("could not convert to number".to_owned())).into())
        }
    }

    fn as_bool(&self) -> Result<bool> {
        let mut scope = self.context.scope();

        let value = self.value.open(&mut scope);
        Ok(value.boolean_value(&mut scope))
    }

    fn as_object(&self) -> Result<<Self::RT as WebRuntime>::Object> {
        let mut scope = self.context.scope();

        let value = self.value.open(&mut scope);

        if let Some(value) = value.to_object(&mut scope) {
            Ok(V8Object::from_ctx(V8Context::clone(&self.context), value))
        } else {
            Err(Error::JS(JSError::Conversion("could not convert to number".to_owned())).into())
        }
    }

    impl_is!(is_string);
    impl_is!(is_number);
    impl_is!(is_object);
    impl_is!(is_array);
    impl_is!(is_null);
    impl_is!(is_undefined);
    impl_is!(is_function);

    fn as_array(&self) -> Result<<Self::RT as WebRuntime>::Array> {
        let value = self.value.clone();

        let mut scope = self.context.scope();

        let value = Local::new(&mut scope, value);

        let array: Local<Array> = value.try_into()?;

        Ok(array.into_ctx(V8Context::clone(&self.context)))
    }

    fn is_bool(&self) -> bool {
        let mut iso = self.context.isolate();

        let value = self.value.open(&mut iso);

        value.is_boolean()
    }

    fn type_of(&self) -> JSType {
        //There is a v8::Value::type_of() method, but it returns a string, which is not what we want.
        //TODO: this currently creates a new scope for each test, this should not be like this
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
            let mut scope = self.context.scope();

            let value = self.value.open(&mut scope);

            let t = value.type_of(&mut scope).to_rust_string_lossy(&mut scope);

            JSType::Other(t)
        }
    }

    fn new_object(ctx: <Self::RT as WebRuntime>::Context) -> Result<<Self::RT as WebRuntime>::Object> {
        V8Object::new(ctx)
    }

    fn new_array<T: IntoWebValue<Self, Value = Self>>(
        ctx: <Self::RT as WebRuntime>::Context,
        value: &[T],
    ) -> Result<<Self::RT as WebRuntime>::Array> {
        value.to_web_array(ctx)
    }

    fn new_empty_array(ctx: <Self::RT as WebRuntime>::Context) -> Result<<Self::RT as WebRuntime>::Array> {
        V8Array::new(ctx, 0)
    }

    fn new_string(ctx: <Self::RT as WebRuntime>::Context, value: &str) -> Result<Self> {
        let scope = &mut ctx.scope();

        if let Some(value) = v8::String::new(scope, value) {
            let value: Local<Value> = value.into();

            Ok(Self {
                context: V8Context::clone(&ctx),
                value: Global::new(scope, value),
            })
        } else {
            Err(Error::JS(JSError::Conversion("could not convert to string".to_owned())).into())
        }
    }

    fn new_number<N: Into<f64>>(ctx: <Self::RT as WebRuntime>::Context, value: N) -> Result<Self> {
        let scope = &mut ctx.scope();

        let value: Local<Value> = v8::Number::new(scope, value.into()).into();
        Ok(Self {
            context: V8Context::clone(&ctx),
            value: Global::new(scope, value),
        })
    }

    fn new_bool(ctx: <Self::RT as WebRuntime>::Context, value: bool) -> Result<Self> {
        let mut isolate = ctx.isolate();

        let value: Local<Value> = v8::Boolean::new(isolate.deref_mut(), value).into();
        Ok(Self {
            context: V8Context::clone(&ctx),
            value: Global::new(&mut isolate, value),
        })
    }

    fn new_null(ctx: <Self::RT as WebRuntime>::Context) -> Result<Self> {
        let mut isolate = ctx.isolate();
        let null: Local<Value> = v8::null(isolate.deref_mut()).into();

        Ok(Self {
            context: V8Context::clone(&ctx),
            value: Global::new(&mut isolate, null),
        })
    }

    fn new_undefined(ctx: <Self::RT as WebRuntime>::Context) -> Result<Self> {
        let mut isolate = ctx.isolate();
        let undefined: Local<Value> = v8::undefined(isolate.deref_mut()).into();

        Ok(Self {
            context: V8Context::clone(&ctx),
            value: Global::new(&mut isolate, undefined),
        })
    }
}

#[cfg(test)]
mod tests {
    use gosub_webexecutor::js::{IntoRustValue, WebContext};

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
