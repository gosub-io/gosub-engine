# gosub_webexecutor

The script-engine abstraction layer of Gosub. This crate defines the trait family that any
JavaScript (or future scripting-language) engine must implement, so the rest of the engine
never names a concrete runtime. It contains no V8 code — `gosub_v8` implements these
traits; this crate only defines them.

> **Status:** the scripting stack is built but not wired into the engine — no page script
> is executed yet. See [docs/javascript.md](../../docs/javascript.md).

## The trait family (`gosub_webexecutor::js`)

- `WebRuntime` — the central trait: 14 associated types, each bound to one of the
  `Web*` traits below, plus `new_context()`.
- `WebContext` — compile + execute (`run(...)`).
- `WebValue`, `WebObject`, `WebArray`, `WebFunction` / `WebFunctionVariadic`,
  `WebCompiled`, getter/setter callbacks, argument types.
- `JSInterop` — the target trait that `gosub_webinterop`-generated glue implements to
  expose a Rust struct into a context.
- `IntoRustValue` / `IntoWebValue` — value conversion in both directions.
- `JSError` / `JSType` — the shared error and type vocabulary.

## Further reading

- [docs/javascript.md](../../docs/javascript.md) — the four scripting crates and how a
  swappable runtime is meant to work
