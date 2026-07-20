# gosub_web_platform

The web event loop for a JS (or Lua) runtime in Gosub — currently an early-stage
scaffold. It drives a dedicated-thread Tokio event loop that a script runtime would run
on: input-event listeners, setTimeout/setInterval-style timers, and a poll hook intended
to drain JS microtasks. It does not embed a script engine itself, and like the rest of
the scripting stack it is **built but not yet wired** into the engine (see
[docs/javascript.md](../../docs/javascript.md)).

## Entry points

- `WebEventLoop<E: FutureExecutor = TokioExecutor>` — the loop;
  `WebEventLoop::new_on_thread()` spawns a current-thread Tokio runtime on its own OS
  thread and returns a handle.
- `WebEventLoopHandle` — spawn tasks on the loop's runtime and send it
  `WebEventLoopMessage`s (input events, close).
- `poll_guard::PollGuard` — a future wrapper that runs a callback on every poll; the
  intended microtask-drain point (currently a stub).

Internals: `event_listeners` (mouse/keyboard listener registry), `timers` (slotmap-based
timer registry on tokio tasks), `callback` (the `FutureExecutor` abstraction).

## Further reading

- [docs/javascript.md](../../docs/javascript.md) — the scripting stack and its
  integration status
