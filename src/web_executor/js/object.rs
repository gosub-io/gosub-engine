use crate::web_executor::js::{JSContext, JSFunction, JSFunctionVariadic, JSValue};
use core::fmt::Display;

pub trait JSObject {
    type Value: JSValue;
    type Function: JSFunction;
    type FunctionVariadic: JSFunctionVariadic;
    type GetterCB: JSGetterCallback;
    type SetterCB: JSSetterCallback;

    fn set_property(&self, name: &str, value: &Self::Value) -> crate::types::Result<()>;

    fn get_property(&self, name: &str) -> crate::types::Result<Self::Value>;

    fn call_method(&self, name: &str, args: &[&Self::Value]) -> crate::types::Result<Self::Value>;

    fn set_method(&self, name: &str, func: &Self::Function) -> crate::types::Result<()>;

    fn set_method_variadic(
        &self,
        name: &str,
        func: &Self::FunctionVariadic,
    ) -> crate::types::Result<()>;

    #[allow(clippy::type_complexity)]
    fn set_property_accessor(
        &self,
        name: &str,
        getter: Box<dyn Fn(&mut Self::GetterCB)>,
        setter: Box<dyn Fn(&mut Self::SetterCB)>,
    ) -> crate::types::Result<()>;
}

pub trait JSGetterCallback {
    type Value: JSValue;
    type Context: JSContext;

    fn context(&mut self) -> &mut Self::Context;

    fn error(&mut self, error: impl Display);

    fn ret(&mut self, value: Self::Value);
}

pub trait JSSetterCallback {
    type Value: JSValue;
    type Context: JSContext;

    fn context(&mut self) -> &mut Self::Context;

    fn error(&mut self, error: impl Display);

    fn value(&mut self) -> &Self::Value;
}
