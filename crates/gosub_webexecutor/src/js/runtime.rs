use gosub_shared::types::Result;

use crate::js::{
    Args, VariadicArgs, VariadicArgsInternal, WebArray, WebCompiled, WebContext, WebFunction, WebFunctionCallBack,
    WebFunctionCallBackVariadic, WebFunctionVariadic, WebGetterCallback, WebObject, WebSetterCallback, WebValue,
};

// trait around the main JS engine (e.g V8, SpiderMonkey, JSC, etc.)
pub trait WebRuntime {
    type Context: WebContext<RT = Self>;
    type Value: WebValue<RT = Self>;
    type Object: WebObject<RT = Self>;
    type Compiled: WebCompiled<RT = Self>;
    type GetterCB: WebGetterCallback<RT = Self>;
    type SetterCB: WebSetterCallback<RT = Self>;
    type Function: WebFunction<RT = Self>;
    type FunctionVariadic: WebFunctionVariadic<RT = Self>;
    type Array: WebArray<RT = Self>;
    type FunctionCallBack: WebFunctionCallBack<RT = Self>;
    type FunctionCallBackVariadic: WebFunctionCallBackVariadic<RT = Self>;
    type Args: Args<RT = Self>;
    type VariadicArgs: VariadicArgs<RT = Self>;
    type VariadicArgsInternal: VariadicArgsInternal<RT = Self>;

    fn new_context(&mut self) -> Result<Self::Context>;
}
