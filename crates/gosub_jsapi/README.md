# gosub_jsapi

Implementations of browser Web APIs for Gosub, written as plain Rust with no knowledge of
any JavaScript engine — the `gosub_webinterop` macros are the intended path for exposing
them into a script context.

Currently exactly one API exists: **`console`**, per the
[WHATWG console spec](https://console.spec.whatwg.org/).

## Entry points

- `console::Console` — `new(Box<dyn Printer>)`; implements `log` / `info` / `warn` /
  `error` / `debug`, `assert`, `trace`, `dir`, counting (`count` / `count_reset`),
  grouping (`group` / `group_collapsed` / `group_end`), and timers (`time` / `time_log` /
  `time_end`).
- `console::Printer` — the pluggable output sink; `WritablePrinter<W>` adapts any
  `std::io::Write`, and `Buffer` is an in-memory sink used by the tests.

## Further reading

- [docs/javascript.md](../../docs/javascript.md) — where the Web APIs sit in the
  scripting stack and the current integration status
