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
cargo build --quiet
target=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
export GENERATED_APP="$target/debug/fleet"
CELL_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
export CELL_PORT
cd "$repo/web"
node generated-cell.mjs
