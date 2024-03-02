use gosub_shared::types::Result;

use crate::js::{
    Args, JSArray, JSCompiled, JSContext, JSFunction, JSFunctionCallBack,
    JSFunctionCallBackVariadic, JSFunctionVariadic, JSGetterCallback, JSObject, JSSetterCallback,
    JSValue, VariadicArgs, VariadicArgsInternal,
};

//trait around the main JS engine (e.g V8, SpiderMonkey, JSC, etc.)
pub trait JSRuntime {
    type Context: JSContext<RT = Self>;
    type Value: JSValue<RT = Self>;
    type Object: JSObject<RT = Self>;
    type Compiled: JSCompiled<RT = Self>;
    type GetterCB: JSGetterCallback<RT = Self>;
    type SetterCB: JSSetterCallback<RT = Self>;
    type Function: JSFunction<RT = Self>;
    type FunctionVariadic: JSFunctionVariadic<RT = Self>;
    type Array: JSArray<RT = Self>;
    type FunctionCallBack: JSFunctionCallBack<RT = Self>;
    type FunctionCallBackVariadic: JSFunctionCallBackVariadic<RT = Self>;
    type Args: Args<RT = Self>;
    type VariadicArgs: VariadicArgs<RT = Self>;
    type VariadicArgsInternal: VariadicArgsInternal<RT = Self>;

    fn new_context(&mut self) -> Result<Self::Context>;
}
