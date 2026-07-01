#!/usr/bin/env bash
#
# Compile every example app, reporting per-example build time, total wall-clock
# time, and the resulting binary size.
#
# Usage: ./compile-all.sh [debug|release]   (default: debug)
#
# Examples are built sequentially so each can be timed on its own. cargo caches
# the shared workspace dependencies, so only the FIRST build pays to compile
# them — the remaining timings reflect each example's own crate. (One combined
# `cargo build -p ... -p ...` would be marginally faster but can't time each.)

set -uo pipefail

cd "$(dirname "$0")"

mode="${1:-debug}"
case "$mode" in
    debug)   profile_flag=();          target_dir="target/debug" ;;
    release) profile_flag=(--release); target_dir="target/release" ;;
    *) echo "Usage: $0 [debug|release]" >&2; exit 1 ;;
esac

# Discover example packages (name = "example-..." in examples/*/Cargo.toml) so
# new examples are picked up automatically.
mapfile -t packages < <(
    find examples -maxdepth 2 -name Cargo.toml \
        -exec grep -hoP '^\s*name\s*=\s*"\Kexample-[^"]+' {} + | sort
)
if [ "${#packages[@]}" -eq 0 ]; then
    echo "No example packages found under examples/" >&2
    exit 1
fi

echo "Compiling ${#packages[@]} examples in ${mode} mode..."
echo
printf '%-26s %9s %9s   %s\n' "EXAMPLE" "TIME" "SIZE" "STATUS"
printf '%-26s %9s %9s   %s\n' "--------------------------" "---------" "---------" "------"

log=$(mktemp)
trap 'rm -f "$log"' EXIT
total_start=$(date +%s.%N)
failures=0

for pkg in "${packages[@]}"; do
    bin="${pkg#example-}"
    printf '%-26s ' "$pkg"   # show the name immediately; the row fills in when done

    start=$(date +%s.%N)
    if cargo build "${profile_flag[@]}" -p "$pkg" >"$log" 2>&1; then
        status="ok"
    else
        status="FAILED"
        failures=$((failures + 1))
    fi
    end=$(date +%s.%N)

    elapsed=$(awk "BEGIN { printf \"%.1fs\", $end - $start }")
    if [ -f "$target_dir/$bin" ]; then
        size=$(du -h "$target_dir/$bin" | cut -f1)
    else
        size="-"
    fi

    printf '%9s %9s   %s\n' "$elapsed" "$size" "$status"
    if [ "$status" = "FAILED" ]; then
        sed 's/^/    | /' "$log"   # indent the cargo error under the row
    fi
done

total_end=$(date +%s.%N)
total=$(awk "BEGIN { printf \"%.1fs\", $total_end - $total_start }")

echo
printf 'Total: %s   (%d examples, %d failed)\n' "$total" "${#packages[@]}" "$failures"

[ "$failures" -eq 0 ]
