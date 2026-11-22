#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --locked --release --target wasm32-unknown-unknown --lib -p paydirt-robot -p paydirt-storage
target_dir=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
for module in paydirt_robot paydirt_storage; do
  wasm-bindgen --remove-name-section --target web --out-dir web/public/pkg "$target_dir/wasm32-unknown-unknown/release/$module.wasm"
done
