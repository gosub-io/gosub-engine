#!/usr/bin/env bash
# Renders every table fixture with gosub-screenshot (Cairo) AND headless
# Chromium, then writes an HTML page showing the pairs side by side.
#
# Usage: scripts/table-compare.sh [output-dir]   (default: target/table-compare)
# Open <output-dir>/compare.html in a browser afterwards.
#
# Notes:
# - Fixtures are served over HTTP because the engine refuses file:// URLs.
# - Snap-confined Chromium can only write inside $HOME (not dot-dirs), so its
#   screenshots go to a temp file in $HOME and are moved into place.
set -euo pipefail

cd "$(dirname "$0")/.."
OUT="${1:-target/table-compare}"
PORT=8734
VIEWPORT=1280
SHOT=target/release/gosub-screenshot
CHROME_BIN="${CHROME_BIN:-chromium}"

mkdir -p "$OUT"

if [ ! -x "$SHOT" ]; then
    echo "building gosub-screenshot (cairo)..."
    cargo build --release -p gosub-screenshot --no-default-features --features backend_cairo
fi

python3 -m http.server "$PORT" --directory tests/data/tables >/dev/null 2>&1 &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT
sleep 0.5

HTML="$OUT/compare.html"
cat > "$HTML" <<'EOF'
<!doctype html><meta charset="utf-8"><title>gosub vs Chrome - table fixtures</title>
<style>
body { font-family: sans-serif; margin: 1rem; background: #f4f4f4; }
h2 { margin: 2rem 0 0.5rem; font-size: 1rem; }
.pair { display: flex; gap: 8px; }
.pair figure { margin: 0; flex: 1; min-width: 0; }
.pair figcaption { font-size: 0.75rem; color: #555; padding: 2px 0; }
.pair img { width: 100%; border: 1px solid #ccc; background: #fff; }
</style>
<h1 style="font-size:1.2rem">gosub_lattice (Cairo) vs Chromium - tests/data/tables</h1>
EOF

CHROME_TMP="$HOME/gosub-table-compare-tmp.png"
find tests/data/tables -name '[0-9][0-9]-*.html' | sort | while read -r fixture; do
    name=$(basename "$fixture" .html)
    echo "== $name"
    "$SHOT" "http://127.0.0.1:$PORT/$name.html" "$OUT/gosub-$name.png" "$VIEWPORT" >/dev/null
    "$CHROME_BIN" --headless --disable-gpu --hide-scrollbars \
        --force-device-scale-factor=1 --window-size="$VIEWPORT,800" \
        --screenshot="$CHROME_TMP" "http://127.0.0.1:$PORT/$name.html" >/dev/null 2>&1
    mv "$CHROME_TMP" "$OUT/chrome-$name.png"
    {
        echo "<h2>$name</h2><div class=\"pair\">"
        echo "<figure><figcaption>gosub</figcaption><img src=\"gosub-$name.png\"></figure>"
        echo "<figure><figcaption>Chrome</figcaption><img src=\"chrome-$name.png\"></figure>"
        echo "</div>"
    } >> "$HTML"
done

echo "wrote $HTML"
