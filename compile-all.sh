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
#
# Portable across Linux (GNU) and macOS (bash 3.2 + BSD find/awk/date).

set -uo pipefail

cd "$(dirname "$0")"

mode="${1:-debug}"
# Plain string (not an array) so expanding it empty is safe under `set -u` on
# bash 3.2 (macOS). Passed unquoted so "" adds no arg and "--release" adds one.
case "$mode" in
    debug)   release_flag="";          target_dir="target/debug" ;;
    release) release_flag="--release"; target_dir="target/release" ;;
    *) echo "Usage: $0 [debug|release]" >&2; exit 1 ;;
esac

# High-resolution clock. GNU `date` supports %N (nanoseconds); BSD/macOS `date`
# does not (leaves an "N"), so fall back to perl, then to integer seconds.
if date +%s.%N 2>/dev/null | grep -q 'N'; then
    if command -v perl >/dev/null 2>&1; then
        now() { perl -MTime::HiRes=time -e 'printf "%.3f", time'; }
    else
        now() { date +%s; }
    fi
else
    now() { date +%s.%N; }
fi

# Examples that only build on Linux — they hard-link libGL and set up a Linux
# OpenGL context (glutin / GTK GLArea), which has no macOS equivalent. They're
# skipped on other platforms instead of failing the run. Space-separated, with a
# leading/trailing space. (If you don't have GTK4 on macOS, add the gtk4-* ones.)
linux_only=" example-winit-skia-gpu example-gtk4-skia-gpu "
os="$(uname -s)"

should_skip() {
    if [ "$os" != "Linux" ]; then
        case "$linux_only" in
            *" $1 "*) return 0 ;;
        esac
    fi
    return 1
}

# Discover example packages: name = "example-..." in examples/*/Cargo.toml, so
# new examples are picked up automatically. awk (not grep -P) keeps this portable.
packages=()
while IFS= read -r name; do
    [ -n "$name" ] && packages+=("$name")
done < <(
    find examples -maxdepth 2 -name Cargo.toml -exec \
        awk -F'"' '/^[[:space:]]*name[[:space:]]*=[[:space:]]*"example-/ { print $2 }' {} + | sort
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
total_start=$(now)
failures=0
skipped=0

for pkg in "${packages[@]}"; do
    bin="${pkg#example-}"
    printf '%-26s ' "$pkg"   # show the name immediately; the row fills in when done

    if should_skip "$pkg"; then
        skipped=$((skipped + 1))
        printf '%9s %9s   %s\n' "-" "-" "skipped (${os})"
        continue
    fi

    start=$(now)
    if cargo build $release_flag -p "$pkg" >"$log" 2>&1; then
        status="ok"
    else
        status="FAILED"
        failures=$((failures + 1))
    fi
    end=$(now)

    elapsed=$(awk "BEGIN { printf \"%.1fs\", $end - $start }")
    if [ -f "$target_dir/$bin" ]; then
        size=$(du -h "$target_dir/$bin" | awk '{ print $1 }')
    else
        size="-"
    fi

    printf '%9s %9s   %s\n' "$elapsed" "$size" "$status"
    if [ "$status" = "FAILED" ]; then
        sed 's/^/    | /' "$log"   # indent the cargo error under the row
    fi
done

total_end=$(now)
total=$(awk "BEGIN { printf \"%.1fs\", $total_end - $total_start }")

echo
printf 'Total: %s   (%d examples, %d failed, %d skipped)\n' \
    "$total" "${#packages[@]}" "$failures" "$skipped"

[ "$failures" -eq 0 ]
