use gosub_shared::types::Result;

use crate::js::{JSArray, JSContext, JSRuntime, JSValue};

//trait to easily convert Rust types to JS values (just call .to_js_value() on the type)
pub trait ValueConversion<V: JSValue> {
    type Value: JSValue;

    fn to_js_value(&self, ctx: <V::RT as JSRuntime>::Context) -> Result<Self::Value>;
}

macro_rules! impl_value_conversion {
    (number, $type:ty) => {
        impl<V: JSValue> ValueConversion<V> for $type {
            type Value = V;

            fn to_js_value(&self, ctx: <V::RT as JSRuntime>::Context) -> Result<Self::Value> {
                Self::Value::new_number(ctx, *self as f64)
            }
        }
    };

    (string, $type:ty) => {
        impl_value_conversion!(new_string, $type, deref);
    };

    (bool, $type:ty) => {
        impl_value_conversion!(new_bool, $type, deref);
    };

    (array, $type:ty) => {
        impl_value_conversion!(new_array, $type, deref);
    };

    ($func:ident, $type:ty, deref) => {
        impl<V: JSValue> ValueConversion<V> for $type {
            type Value = V;
            fn to_js_value(&self, ctx: <V::RT as JSRuntime>::Context) -> Result<Self::Value> {
                Self::Value::$func(ctx, *self)
            }
        }
    };

    ($func:ident, $type:ty) => {
        impl<C: JSContext> ValueConversion<C> for $type {
            type Context = C;

            fn to_js_value(&self, ctx: C) -> Result<C::Value> {
                C::Value::$func(ctx, self)
            }
        }
    };
}

impl_value_conversion!(number, i8);
impl_value_conversion!(number, i16);
impl_value_conversion!(number, i32);
impl_value_conversion!(number, i64);
impl_value_conversion!(number, isize);
impl_value_conversion!(number, u8);
impl_value_conversion!(number, u16);
impl_value_conversion!(number, u32);
impl_value_conversion!(number, u64);
impl_value_conversion!(number, usize);
impl_value_conversion!(number, f32);
impl_value_conversion!(number, f64);

impl_value_conversion!(string, &str);

impl_value_conversion!(bool, bool);

impl<V: JSValue> ValueConversion<V> for String {
    type Value = V;
    fn to_js_value(&self, ctx: <V::RT as JSRuntime>::Context) -> Result<Self::Value> {
        Self::Value::new_string(ctx, self)
    }
}

impl<V: JSValue> ValueConversion<V> for () {
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
    T: ValueConversion<<A::RT as JSRuntime>::Value, Value = <A::RT as JSRuntime>::Value>,
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
