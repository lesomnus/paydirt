#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export CELL_PROJECT="$work/fleet" CELL_SQLITE_PATH="$work/app.sqlite3"
"${PAYDIRT_BIN:-$repo/target/debug/paydirt}" new fleet "$CELL_PROJECT"
# Add the upstream declaration without changing the generated application’s Rust code.
python3 - "$repo/web/upstream/internal/apptest/proto/app/robot.proto" "$CELL_PROJECT/proto/app/cell.proto" <<'PY'
from pathlib import Path
import sys
source=Path(sys.argv[1]).read_text()
cell=source.split("message Cell {",1)[1].split("\n}",1)[0]
Path(sys.argv[2]).write_text('edition = "2023";\npackage app;\nimport "orm.proto";\nimport "payday.proto";\nimport "app/payday/tenant.proto";\nimport "google/protobuf/timestamp.proto";\noption features.field_presence = IMPLICIT;\noption go_package = "example.org/app";\nmessage Cell {'+cell+'\n}\n')
PY
"${PAYDIRT_BIN:-$repo/target/debug/paydirt}" gen "$CELL_PROJECT"
cd "$CELL_PROJECT"
python3 -c 'import json,pathlib;p=pathlib.Path("app.json");c=json.loads(p.read_text());c["allow_general_writes"]=True;p.write_text(json.dumps(c))'
cargo build --quiet
target=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
export GENERATED_APP="$target/debug/fleet"
CELL_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
export CELL_PORT
cd "$repo/web"
node generated-cell.mjs

# Install the application-owned overlay and its single Rust handler into this same starter.
python3 "$repo/scripts/install-fleet-core.py" "$CELL_PROJECT"
cp "$CELL_PROJECT/src/robot.rs" "$work/original.rs"
"$repo/target/debug/paydirt" gen "$CELL_PROJECT"
"$repo/target/debug/paydirt" gen --check "$CELL_PROJECT"
cmp "$work/original.rs" "$CELL_PROJECT/src/robot.rs"
cargo build --quiet --manifest-path "$CELL_PROJECT/Cargo.toml"
export PAYDIRT_TEST_MOVE=1 CELL_SQLITE_PATH="$work/fleet.sqlite3"
node generated-cell.mjs
node generated-replicas.mjs
node generated-grant-follower.mjs
if [[ "${PAYDIRT_TEST_WASM:-0}" == 1 ]]; then
  "${PAYDIRT_BIN:-$repo/target/debug/paydirt}" sandbox init "$CELL_PROJECT"
  "${PAYDIRT_BIN:-$repo/target/debug/paydirt}" sandbox init "$CELL_PROJECT"
  (cd "$CELL_PROJECT/sandbox" && bash build.sh && npm ci --no-audit --no-fund)
  if [[ -n "${PAYDIRT_GO_CHECKOUT:-}" ]]; then
    (cd "$PAYDIRT_GO_CHECKOUT/internal/apptest" && GOOS=js GOARCH=wasm go build -o "$CELL_PROJECT/sandbox/public/stream-reference.wasm" "$repo/tests/fleet/stream-reference.go")
    cp "$(go env GOROOT)/lib/wasm/wasm_exec.js" "$CELL_PROJECT/sandbox/public/"
  fi
  node generated-browser.mjs
fi

if [[ -n "${PAYDIRT_GO_CHECKOUT:-}" ]]; then
  "$repo/scripts/test-sdk.sh"
fi
