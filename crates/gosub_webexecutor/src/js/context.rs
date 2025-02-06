use gosub_shared::types::Result;

use crate::js::WebRuntime;

//main trait for JS context (can be implemented for different JS engines like V8, SpiderMonkey, JSC, etc.)
pub trait WebContext: Clone {
    type RT: WebRuntime<Context = Self>;
    fn run(&mut self, code: &str) -> Result<<Self::RT as WebRuntime>::Value>;

    fn compile(&mut self, code: &str) -> Result<<Self::RT as WebRuntime>::Compiled>;

    fn run_compiled(
        &mut self,
        compiled: &mut <Self::RT as WebRuntime>::Compiled,
    ) -> Result<<Self::RT as WebRuntime>::Value>;

    // fn compile_stream(&self, code: &str) -> Result<()>;

    // fn new_global_object(&mut self, name: &str) -> Result<<Self::RT as WebRuntime>::Object>;

    fn set_on_global_object(
        &mut self,
        name: &str, //TODO: this should be impl IntoWebValue
        value: <Self::RT as WebRuntime>::Value,
    ) -> Result<()>;
}
