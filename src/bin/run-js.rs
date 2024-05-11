#![allow(clippy::unwrap_used, clippy::expect_used)]
use gosub_shared::types::Result;
use gosub_v8::{V8Context, V8Engine};
use gosub_webexecutor::js::{JSContext, JSRuntime, JSValue};
use std::env::args;

fn main() -> Result<()> {
    let file = args().nth(1).expect("no file given");

    let mut runtime = V8Engine::new();
    let mut ctx: V8Context = runtime.new_context()?;

    let code = std::fs::read_to_string(file)?;

    let value = ctx.run(&code)?;

    println!("Got Value: {}", value.as_string()?);

    Ok(())
}
