#!/bin/sh
# Linker selector for the workspace.
#
# Uses mold (via clang) when BOTH mold and clang are available - it is several
# times faster than the default linker on the big example/screenshot binaries.
# Falls back to the system default linker (cc) otherwise, so the build works for
# every developer whether or not mold is installed.
#
# mold only produces ELF, so it is used on Linux only; on macOS (Mach-O) and
# elsewhere we always use the system default linker.
#
# Referenced from .cargo/config.toml as the [target.*] linker.

if [ "$(uname -s)" = "Linux" ] && command -v mold >/dev/null 2>&1 && command -v clang >/dev/null 2>&1; then
    exec clang -fuse-ld=mold "$@"
fi

exec cc "$@"
