#!/usr/bin/bash
set -euo pipefail

if [ -z "${1:-}" ]; then
    echo "Usage: $0 <url>" >&2
    exit 1
fi

. tools/souper/bin/activate
python tools/souper/soupertoo.py "$1"

cargo run
