#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
repo=$(pwd)
out="$repo/sandbox/public/pkg"
build() {
  local manifest=$1 module=$2 name=$3
  cargo build --manifest-path "$manifest" --release --target wasm32-unknown-unknown --lib
  local target
  target=$(cargo metadata --manifest-path "$manifest" --no-deps --format-version 1 | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')
  wasm-bindgen --target web --remove-name-section --out-dir "$out" --out-name "$name" "$target/wasm32-unknown-unknown/release/$module.wasm"
}
module=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(t["name"] for p in json.load(sys.stdin)["packages"] for t in p["targets"] if "cdylib" in t["crate_types"]))')
build Cargo.toml "$module" app
build sandbox/database/Cargo.toml paydirt_app_database storage
cp generated/storage/descriptors.binpb "$out/descriptors.binpb"
python3 - "$out" <<'PYTHON'
import hashlib,json,sys
from pathlib import Path
directory=Path(sys.argv[1])
manifest={}
for name in ("app","storage"):
    data=(directory/(name+"_bg.wasm")).read_bytes()
    manifest[name]={"bytes":len(data),"sha256":hashlib.sha256(data).hexdigest()}
(directory/"modules.json").write_text(json.dumps(manifest))
PYTHON
wc -c "$out"/{app,storage}_bg.wasm
