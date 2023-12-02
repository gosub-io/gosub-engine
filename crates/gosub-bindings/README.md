# Gosub Bindings
These are bindings that expose some of Gosub's engine to the world via a C API. Typically these bindings will be used by user agents.

## Building
By default, the bindings will be built in release mode. You can modify this by specifying a `MODE` variable:
```text
export MODE=Debug # or MODE=Release (default)
make bindings
make test
```

or alternatively specify it manually (not recommended)
```text
make bindings MODE=Debug
make test MODE=Debug
```

This approach is not recommended because if you forget to specify it, it will default to release mode and you may be using the wrong version.
