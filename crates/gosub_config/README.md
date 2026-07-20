# gosub_config

The runtime settings store of the Gosub engine. Schema-agnostic by design: the caller
supplies the schema (known keys, defaults, constraints) and only known keys can be read
or written — this crate ships no settings of its own. The engine's schema lives in
`gosub_engine` (`settings.json` / `useragent-settings.json`), which seeds a `Config` via
`gosub_engine::default_settings()`.

Not to be confused with compile-time component selection (`ModuleConfiguration` /
`DefaultRenderConfig`) — that is a different mechanism, documented in
[docs/configuration.md](../../docs/configuration.md). This crate is for values that
change at runtime.

## Entry points

- `Config` — cheap-to-clone handle (`Arc<RwLock<ConfigStore>>`); `Config::new(schema)`
  or `with_storage(schema, storage)`. Typed getters (`get_bool`, `get_uint`, ...),
  wildcard `find`, namespaced `merge`, and change **subscriptions**
  (`subscribe`/`unsubscribe` with wildcard patterns).
- `StorageAdapter` — the pluggable persistence trait, with three implementations:
  `MemoryStorageAdapter` (default), `JsonStorageAdapter`, and `SqliteStorageAdapter`
  (not available on wasm32).
- `settings::{Setting, SettingInfo, Constraint}` — the value model: typed settings with
  a wire format (`b:true`, `u:1000`, `s:...`) and constraints (enums, numeric ranges).
- `HasConfig` — accessor bound so subsystems depend on `T: HasConfig` rather than a
  concrete context type.

## Trying it

The `config-store` example in the workspace root demonstrates the store:
`cargo run --example config-store`.

## Further reading

- [docs/configuration.md](../../docs/configuration.md) — the two configuration layers
  and where this store fits
