use crate::js::{JSContext, JSValue};
use crate::types::Result;

//trait to easily convert Rust types to JS values (just call .to_js_value() on the type)
pub trait ValueConversion<V: JSValue, C: JSContext> {
    type Value: JSValue;

    type Context: JSContext;

    fn to_js_value(&self, ctx: Self::Context) -> Result<Self::Value>;
}

macro_rules! impl_value_conversion {
    (number, $type:ty) => {
        impl<V: JSValue<Context = C>, C: JSContext> ValueConversion<V, C> for $type {
            type Value = V;
            type Context = C;

            fn to_js_value(&self, ctx: Self::Context) -> Result<Self::Value> {
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
        impl<V: JSValue<Context = C>, C: JSContext> ValueConversion<V, C> for $type {
            type Value = V;
            type Context = C;

            fn to_js_value(&self, ctx: Self::Context) -> Result<Self::Value> {
                Self::Value::$func(ctx, *self)
            }
        }
    };

    ($func:ident, $type:ty) => {
        impl<V: JSValue<Context = C>, C: JSContext> ValueConversion<V, C> for $type {
            type Value = V;
            type Context = C;

            fn to_js_value(&self, ctx: Self::Context) -> Result<Self::Value> {
                Self::Value::$func(ctx, self)
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

impl<V: JSValue<Context = C>, C: JSContext> ValueConversion<V, C> for String {
    type Value = V;
    type Context = C;

    fn to_js_value(&self, ctx: Self::Context) -> Result<Self::Value> {
        Self::Value::new_string(ctx, self)
    }
}
