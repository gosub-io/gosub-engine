use core::fmt::Display;

use gosub_shared::types::Result;

use crate::js::JSRuntime;

pub trait JSObject: Into<<Self::RT as JSRuntime>::Value> {
    type RT: JSRuntime<Object = Self>;

    fn set_property(&self, name: &str, value: &<Self::RT as JSRuntime>::Value) -> Result<()>;

    fn get_property(&self, name: &str) -> Result<<Self::RT as JSRuntime>::Value>;

    fn call_method(
        &self,
        name: &str,
        args: &[&<Self::RT as JSRuntime>::Value],
    ) -> Result<<Self::RT as JSRuntime>::Value>;

    fn set_method(&self, name: &str, func: &<Self::RT as JSRuntime>::Function) -> Result<()>;

    fn set_method_variadic(
        &self,
        name: &str,
        func: &<Self::RT as JSRuntime>::FunctionVariadic,
    ) -> Result<()>;

    #[allow(clippy::type_complexity)]
    fn set_property_accessor(
        &self,
        name: &str,
        getter: Box<dyn Fn(&mut <Self::RT as JSRuntime>::GetterCB)>,
        setter: Box<dyn Fn(&mut <Self::RT as JSRuntime>::SetterCB)>,
    ) -> Result<()>;
}

pub trait JSGetterCallback {
    type RT: JSRuntime<GetterCB = Self>;

    fn context(&mut self) -> &mut <Self::RT as JSRuntime>::Context;

    fn error(&mut self, error: impl Display);

    fn ret(&mut self, value: <Self::RT as JSRuntime>::Value);
}

pub trait JSSetterCallback {
    type RT: JSRuntime<SetterCB = Self>;

    fn context(&mut self) -> &mut <Self::RT as JSRuntime>::Context;

    fn error(&mut self, error: impl Display);

    fn value(&mut self) -> &<Self::RT as JSRuntime>::Value;
}
