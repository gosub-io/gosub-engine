use gosub_shared::types::Result;

use crate::js::JSRuntime;

//main trait for JS context (can be implemented for different JS engines like V8, SpiderMonkey, JSC, etc.)
pub trait JSContext: Clone {
    type RT: JSRuntime<Context = Self>;
    fn run(&mut self, code: &str) -> Result<<Self::RT as JSRuntime>::Value>;

    fn compile(&mut self, code: &str) -> Result<<Self::RT as JSRuntime>::Compiled>;

    fn run_compiled(
        &mut self,
        compiled: &mut <Self::RT as JSRuntime>::Compiled,
    ) -> Result<<Self::RT as JSRuntime>::Value>;

    // fn compile_stream(&self, code: &str) -> Result<()>;

    fn new_global_object(&mut self, name: &str) -> Result<<Self::RT as JSRuntime>::Object>;
}
