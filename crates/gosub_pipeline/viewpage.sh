#!/usr/bin/bash
set -euo pipefail

. tools/souper/bin/activate
python tools/souper/soupertoo.py "$1"

cargo run
