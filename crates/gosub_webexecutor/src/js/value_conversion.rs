use gosub_shared::types::Result;

use crate::js::{JSArray, JSContext, JSError, JSRuntime, JSValue};

//trait to easily convert Rust types to JS values (just call .to_js_value() on the type)
pub trait IntoJSValue<V: JSValue> {
    type Value: JSValue;

    fn to_js_value(&self, ctx: <V::RT as JSRuntime>::Context) -> Result<Self::Value>;
}

macro_rules! impl_value_conversion {
    (number, $type:ty) => {
        impl<V: JSValue> IntoJSValue<V> for $type {
            type Value = V;

            fn to_js_value(&self, ctx: <V::RT as JSRuntime>::Context) -> Result<Self::Value> {
                Self::Value::new_number(ctx, *self as f64)
            }
        }
    };

    ($func:ident, $type:ty) => {
        impl<V: JSValue> IntoJSValue<V> for $type {
            type Value = V;
            fn to_js_value(&self, ctx: <V::RT as JSRuntime>::Context) -> Result<Self::Value> {
                Self::Value::$func(ctx, *self)
            }
        }
    };
}

impl_value_conversion!(number, i8);
impl_value_conversion!(number, i16);
impl_value_conversion!(number, i32);
impl_value_conversion!(number, i64);
impl_value_conversion!(number, isize);
impl_value_conversion!(number, i128);
impl_value_conversion!(number, u8);
impl_value_conversion!(number, u16);
impl_value_conversion!(number, u32);
impl_value_conversion!(number, u64);
impl_value_conversion!(number, usize);
impl_value_conversion!(number, u128);
impl_value_conversion!(number, f32);
impl_value_conversion!(number, f64);

impl_value_conversion!(new_string, &str);

impl_value_conversion!(new_bool, bool);

impl<V: JSValue> IntoJSValue<V> for String {
    type Value = V;
    fn to_js_value(&self, ctx: <V::RT as JSRuntime>::Context) -> Result<Self::Value> {
        Self::Value::new_string(ctx, self)
    }
}

impl<V: JSValue> IntoJSValue<V> for () {
    type Value = V;
    fn to_js_value(&self, ctx: <V::RT as JSRuntime>::Context) -> Result<Self::Value> {
        Self::Value::new_undefined(ctx)
    }
}

pub trait ArrayConversion<A: JSArray> {
    type Array: JSArray;

    fn to_js_array(&self, ctx: <A::RT as JSRuntime>::Context) -> Result<A>;
}

impl<A, T> ArrayConversion<A> for [T]
where
    A: JSArray,
    T: IntoJSValue<<A::RT as JSRuntime>::Value, Value = <A::RT as JSRuntime>::Value>,
{
    type Array = A;
    fn to_js_array(&self, ctx: <A::RT as JSRuntime>::Context) -> Result<A> {
        let data = self
            .iter()
            .map(|v| v.to_js_value(ctx.clone()))
            .collect::<Result<Vec<_>>>()?;

        Self::Array::new_with_data(ctx.clone(), &data)
    }
}


pub trait IntoRustValue<T> {
    fn to_rust_value(&self) -> Result<T>
    where
        Self: Sized;
}

macro_rules! impl_rust_conversion {
    ($func:ident, $type:ty, cast) => {
        impl<T: JSValue> IntoRustValue<$type> for T {
            fn to_rust_value(&self) -> Result<$type> {
                Ok(self.$func()? as $type)
            }
        }
    };

    ($func:ident, $type:ty) => {
        impl<T: JSValue> IntoRustValue<$type> for T {
            fn to_rust_value(&self) -> Result<$type> {
                self.$func()
            }
        }
    };
}

impl_rust_conversion!(as_number, i8, cast);
impl_rust_conversion!(as_number, i16, cast);
impl_rust_conversion!(as_number, i32, cast);
impl_rust_conversion!(as_number, i64, cast);
impl_rust_conversion!(as_number, isize, cast);
impl_rust_conversion!(as_number, i128, cast);
impl_rust_conversion!(as_number, u8, cast);
impl_rust_conversion!(as_number, u16, cast);
impl_rust_conversion!(as_number, u32, cast);
impl_rust_conversion!(as_number, u64, cast);
impl_rust_conversion!(as_number, usize, cast);
impl_rust_conversion!(as_number, u128, cast);
impl_rust_conversion!(as_number, f32, cast);
impl_rust_conversion!(as_number, f64, cast);

impl_rust_conversion!(as_string, String);
impl_rust_conversion!(as_bool, bool);

impl<T: JSValue> IntoRustValue<()> for T {
    fn to_rust_value(&self) -> Result<()> {
        if self.is_undefined() || self.is_null() {
            Ok(())
        } else {
            Err(JSError::Conversion("Value is not undefined or null".to_string()).into())
        }
    }
}
