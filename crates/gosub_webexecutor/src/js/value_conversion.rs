use paste;

use gosub_shared::types::Result;

use crate::js::{JSError, WebArray, WebRuntime, WebValue};

//trait to easily convert Rust types to JS values (just call .to_web_value() on the type)
pub trait IntoWebValue<V: WebValue> {
    type Value: WebValue;

    fn to_web_value(&self, ctx: <V::RT as WebRuntime>::Context) -> Result<V>;
}

macro_rules! impl_value_conversion {
    (number, $type:ty) => {
        impl<V: WebValue> IntoWebValue<V> for $type {
            type Value = V;

            fn to_web_value(&self, ctx: <V::RT as WebRuntime>::Context) -> Result<Self::Value> {
                Self::Value::new_number(ctx, *self as f64)
            }
        }
    };

    ($func:ident, $type:ty) => {
        impl<V: WebValue> IntoWebValue<V> for $type {
            type Value = V;
            fn to_web_value(&self, ctx: <V::RT as WebRuntime>::Context) -> Result<Self::Value> {
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

impl<V: WebValue> IntoWebValue<V> for String {
    type Value = V;
    fn to_web_value(&self, ctx: <V::RT as WebRuntime>::Context) -> Result<Self::Value> {
        Self::Value::new_string(ctx, self)
    }
}

impl<V: WebValue> IntoWebValue<V> for () {
    type Value = V;
    fn to_web_value(&self, ctx: <V::RT as WebRuntime>::Context) -> Result<Self::Value> {
        Self::Value::new_undefined(ctx)
    }
}

pub trait ArrayConversion<A: WebArray> {
    type Array: WebArray;

    fn to_web_array(&self, ctx: <A::RT as WebRuntime>::Context) -> Result<A>;
}

impl<A, T> ArrayConversion<A> for [T]
where
    A: WebArray,
    T: IntoWebValue<<A::RT as WebRuntime>::Value, Value = <A::RT as WebRuntime>::Value>,
{
    type Array = A;
    fn to_web_array(&self, ctx: <A::RT as WebRuntime>::Context) -> Result<A> {
        let data = self
            .iter()
            .map(|v| v.to_web_value(ctx.clone()))
            .collect::<Result<Vec<_>>>()?;

        Self::Array::new_with_data(ctx.clone(), &data)
    }
}

impl<V, T> IntoWebValue<V> for [T]
where
    V: WebValue,
    T: IntoWebValue<V, Value = V>,
    V::RT: WebRuntime<Value = V>,
{
    type Value = V;
    fn to_web_value(&self, ctx: <V::RT as WebRuntime>::Context) -> Result<Self::Value> {
        let data = self
            .iter()
            .map(|v| v.to_web_value(ctx.clone()))
            .collect::<Result<Vec<_>>>()?;

        <V::RT as WebRuntime>::Array::new_with_data(ctx.clone(), &data).map(|v| v.as_value())
    }
}

pub trait IntoRustValue<T> {
    fn to_rust_value(&self) -> Result<T>
    where
        Self: Sized;
}

macro_rules! impl_rust_conversion {
    ($func:ident, $type:ty, cast) => {
        impl<T: WebValue> IntoRustValue<$type> for T {
            fn to_rust_value(&self) -> Result<$type> {
                Ok(self.$func()? as $type)
            }
        }
    };

    ($func:ident, $type:ty) => {
        impl<T: WebValue> IntoRustValue<$type> for T {
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

impl<T: WebValue> IntoRustValue<()> for T {
    fn to_rust_value(&self) -> Result<()> {
        if self.is_undefined() || self.is_null() {
            Ok(())
        } else {
            Err(JSError::Conversion("Value is not undefined or null".to_string()).into())
        }
    }
}

pub enum Ref<'a, T> {
    //basically cow but without clone
    Ref(&'a T),
    Owned(T),
}

impl<T> Ref<'_, T> {
    fn get_ref(&self) -> &T {
        match self {
            Ref::Ref(r) => r,
            Ref::Owned(r) => r,
        }
    }
}

pub trait AsArray {
    type Runtime: WebRuntime;
    fn array(&self) -> Result<Ref<<Self::Runtime as WebRuntime>::Array>>;
}

impl<V, T> IntoRustValue<Vec<T>> for V
where
    V: AsArray,
    <V::Runtime as WebRuntime>::Value: IntoRustValue<T>,
{
    fn to_rust_value(&self) -> Result<Vec<T>> {
        let arr = self.array()?;
        let arr = arr.get_ref();
        let mut vec: Vec<T> = Vec::with_capacity(arr.len());
        for i in 0..arr.len() {
            vec.push(arr.get(i)?.to_rust_value()?);
        }
        Ok(vec)
    }
}
macro_rules! impl_tuple {
    ($($t:expr),*) => {
            paste::paste! {
                impl<V: WebValue, $([<T $t>]: IntoWebValue<<V::RT as WebRuntime>::Value>),*> IntoWebValue<V> for ($([<T $t>],)*)
                {
                    type Value = V;

                    fn to_web_value(&self, ctx: <V::RT as WebRuntime>::Context) -> Result<Self::Value> {
                        let vals = vec![$(self.$t.to_web_value(ctx.clone())?),*];
                        let arr = <V::RT as WebRuntime>::Array::new_with_data(ctx, &vals)?;
                        Ok(arr.as_value())
                    }
                }

                impl<V: AsArray, $([<T $t>]),*> IntoRustValue<($([<T $t>],)*)> for V
                where
                    $(<V::Runtime as WebRuntime>::Value: IntoRustValue<[<T $t>]>),*

                {
                    fn to_rust_value(&self) -> Result<($([<T $t>],)*)> {
                        let arr = self.array()?;
                        let arr = arr.get_ref();
                        Ok(($((arr.get($t)?.to_rust_value()?),)*))
                    }
                }
            }
    };
}

impl_tuple!(0);
impl_tuple!(0, 1);
impl_tuple!(0, 1, 2);
impl_tuple!(0, 1, 2, 3);
impl_tuple!(0, 1, 2, 3, 4);
impl_tuple!(0, 1, 2, 3, 4, 5);
impl_tuple!(0, 1, 2, 3, 4, 5, 6);
impl_tuple!(0, 1, 2, 3, 4, 5, 6, 7);
impl_tuple!(0, 1, 2, 3, 4, 5, 6, 7, 8);
impl_tuple!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9);
impl_tuple!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10);
impl_tuple!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11);
