use crate::types::Result;
use crate::web_executor::js::v8::V8Engine;
use crate::web_executor::js::{
    Args, JSArray, JSCompiled, JSContext, JSFunction, JSFunctionCallBack,
    JSFunctionCallBackVariadic, JSFunctionVariadic, JSObject, JSValue, VariadicArgsInternal,
};

//trait around the main JS engine (e.g V8, SpiderMonkey, JSC, etc.)
pub trait JSRuntime {
    type Context: JSContext;
    fn new_context(&mut self) -> Result<Self::Context>;
}
