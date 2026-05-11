#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "Usage: $0 <url>" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "${BASH_SOURCE[0]%/*}" && pwd)"

. "$SCRIPT_DIR/tools/souper/bin/activate"
python3 "$SCRIPT_DIR/tools/souper/soupertoo.py" "$1"

cargo run --manifest-path "$SCRIPT_DIR/Cargo.toml"
