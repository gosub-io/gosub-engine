use crate::js::{JSCompiled, JSObject, JSValue};

//main trait for JS context (can be implemented for different JS engines like V8, SpiderMonkey, JSC, etc.)
pub trait JSContext {
    type Object: JSObject;

    type Value: JSValue;

    type Compiled: JSCompiled;

    fn run(&mut self, code: &str) -> crate::types::Result<Self::Value>;

    fn compile(&mut self, code: &str) -> crate::types::Result<Self::Compiled>;

    fn run_compiled(&mut self, compiled: &mut Self::Compiled) -> crate::types::Result<Self::Value>;

    // fn compile_stream(&self, code: &str) -> Result<()>;

    fn new_global_object(&mut self, name: &str) -> crate::types::Result<Self::Object>;
}

//wrapper for JSContext to allow for multiple JS engines to be used - probably not needed TODO: remove
pub struct Context<C: JSContext>(pub C);

impl<T> JSContext for Context<T>
where
    T: JSContext,
{
    type Object = T::Object;
    type Value = T::Value;

    type Compiled = T::Compiled;

    fn run(&mut self, code: &str) -> crate::types::Result<Self::Value> {
        self.0.run(code)
    }

    fn compile(&mut self, code: &str) -> crate::types::Result<Self::Compiled> {
        self.0.compile(code)
    }

    fn run_compiled(&mut self, compiled: &mut Self::Compiled) -> crate::types::Result<Self::Value> {
        self.0.run_compiled(compiled)
    }

    fn new_global_object(&mut self, name: &str) -> crate::types::Result<Self::Object> {
        self.0.new_global_object(name)
    }
}
