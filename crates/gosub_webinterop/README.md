# gosub_webinterop

Proc-macro crate that generates the Rust ↔ JavaScript marshalling glue for Gosub Web APIs,
so crates like `gosub_jsapi` never hand-write binding code. The generated code targets the
runtime-agnostic `WebRuntime` / `JSInterop` traits from `gosub_webexecutor`, not V8
directly — any conforming runtime can consume it (today the only one is `gosub_v8`).

## The macros

- `#[web_interop]` — on a struct: exposes `#[property]`-annotated fields with generated
  getters/setters (supports `rename`, `executor`, and a `js_name = "..."` argument).
- `#[web_fns]` — on an `impl` block: wraps each method into a JS-callable function.
  Handles `&self` / `&mut self` / free functions, generics, `rename`, variadic arguments
  (must be last), and an optional trailing `Context` argument.

## Testing

The end-to-end tests live in `gosub_v8` (`src/tests/interop.rs`), which drives an
annotated struct through V8: methods, `Vec`s, slices, tuples, and nested arrays.

## Further reading

- [docs/javascript.md](../../docs/javascript.md) — the bindings generator in context of
  the full scripting stack
