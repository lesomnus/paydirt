#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
checkout=$(cd "${PAYDAY_CHECKOUT:?path to the pinned Go Payday checkout}" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
python3 - "$checkout" "$repo" "$work/overlay.json" <<'PY'
import json,sys,pathlib
go,repo,out=map(pathlib.Path,sys.argv[1:])
out.write_text(json.dumps({"Replace":{str(go/"trail/paydirt_vectors_test.go"):str(repo/"tests/trail/vectors_test.go")}}))
PY
(cd "$checkout" && PAYDIRT_TRAIL_VECTORS="$repo/tests/trail/go-vectors.json" go test -overlay "$work/overlay.json" ./trail -run '^TestPaydirtVectors$' -count=1)
