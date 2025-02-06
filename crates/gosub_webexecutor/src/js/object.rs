use core::fmt::Display;

use gosub_shared::types::Result;

use crate::js::WebRuntime;

pub trait WebObject: Into<<Self::RT as WebRuntime>::Value> + Clone {
    type RT: WebRuntime<Object = Self>;

    fn set_property(&self, name: &str, value: &<Self::RT as WebRuntime>::Value) -> Result<()>;

    fn get_property(&self, name: &str) -> Result<<Self::RT as WebRuntime>::Value>;

    fn call_method(
        &self,
        name: &str,
        args: &[&<Self::RT as WebRuntime>::Value],
    ) -> Result<<Self::RT as WebRuntime>::Value>;

    fn set_method(&self, name: &str, func: &<Self::RT as WebRuntime>::Function) -> Result<()>;

    fn set_method_variadic(&self, name: &str, func: &<Self::RT as WebRuntime>::FunctionVariadic) -> Result<()>;

    #[allow(clippy::type_complexity)]
    fn set_property_accessor(
        &self,
        name: &str,
        getter: Box<dyn Fn(&mut <Self::RT as WebRuntime>::GetterCB)>,
        setter: Box<dyn Fn(&mut <Self::RT as WebRuntime>::SetterCB)>,
    ) -> Result<()>;

    fn new(ctx: &<Self::RT as WebRuntime>::Context) -> Result<Self>;
}

pub trait WebGetterCallback {
    type RT: WebRuntime<GetterCB = Self>;

    fn context(&mut self) -> &mut <Self::RT as WebRuntime>::Context;

    fn error(&mut self, error: impl Display);

    fn ret(&mut self, value: <Self::RT as WebRuntime>::Value);
}

pub trait WebSetterCallback {
    type RT: WebRuntime<SetterCB = Self>;

    fn context(&mut self) -> &mut <Self::RT as WebRuntime>::Context;

    fn error(&mut self, error: impl Display);

    fn value(&mut self) -> &<Self::RT as WebRuntime>::Value;
}
