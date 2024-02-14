use gosub_shared::types::Result;
use gosub_js::web_executor::js::v8::V8Context;
use gosub_js::web_executor::js::{JSContext, JSRuntime, JSValue, RUNTIME};
use std::env::args;

fn main() -> Result<()> {
    let file = args().nth(1).expect("no file given");

    let mut ctx: V8Context = RUNTIME.lock().unwrap().new_context()?;

    let code = std::fs::read_to_string(file)?;

    let value = ctx.run(&code)?;

    println!("Got Value: {}", value.as_string()?);

    Ok(())
}
