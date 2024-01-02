use gosub_engine::js::v8::V8Context;
use gosub_engine::js::{Context, JSContext, JSRuntime, JSValue, RUNTIME};
use gosub_engine::types::Result;
use std::env::args;

fn main() -> Result<()> {
    let file = args().nth(1).expect("no file given");

    let mut ctx: Context<V8Context> = RUNTIME.lock().unwrap().new_context()?;

    let code = std::fs::read_to_string(file)?;

    let value = ctx.run(&code)?;

    println!("Got Value: {}", value.as_string()?);

    Ok(())
}
