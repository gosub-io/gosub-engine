# gosub_v8

Rust bindings to the V8 JavaScript engine for Gosub. This crate wraps the `v8` crate and
implements the runtime-agnostic `WebRuntime` trait family from `gosub_webexecutor` — it is
currently the only script-engine implementation in the workspace.

> **Status:** the scripting stack is built but not wired into the engine — no page script
> is executed yet. See [docs/javascript.md](../../docs/javascript.md).

## Entry points

- `V8Engine` — zero-sized engine handle; `V8Engine::new()` initializes the V8 platform
  exactly once (guarded, with a fail-fast timeout).
- `V8Context` — the execution context wrapper (`with_default()`, `run(...)` via the
  `WebContext` trait). Everything in V8 needs a context, so the crate threads one through
  its internal `FromContext` / `IntoContext` conversion traits.
- The `WebRuntime` impl maps every associated type: `V8Value`, `V8Object`, `V8Function`,
  `V8FunctionVariadic`, `V8Array`, `V8Compiled`, argument and callback types.

## Structure

One module per trait implementation under `src/v8/`: `context`, `value`, `object`,
`array`, `function`, `compile`. The `gosub_webinterop` proc-macros are exercised
end-to-end against V8 in `src/tests/interop.rs` (which is why that crate appears as a
dev-dependency).

This is the only workspace crate with crate-wide `unsafe_code = "allow"` (it is an FFI
layer; a handful of others relax the workspace `forbid` to `deny` for isolated cases).
The panic-family clippy lints remain denied.

## Trying it

The `run-js` binary lives in the root package: `cargo run --bin run-js -- script.js`.

## Further reading

- [docs/javascript.md](../../docs/javascript.md) — how the four scripting crates stack
  together and the current integration status
- [docs/binaries.md](../../docs/binaries.md) — the `run-js` tool
