#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
rev=53acc135cc084adafda524e882f19275c0d0cc96
paths=(internal/apptest/ts/gen ts/src/sandbox/link.ts ts/src/pderr/index.ts ts/gen/google/rpc/error_details_pb.ts internal/apptest/proto proto buf.yaml buf.lock LICENSE)
mkdir -p web/upstream
if [[ -n "${PAYDAY_CHECKOUT:-}" ]]; then
  git -C "$PAYDAY_CHECKOUT" archive "$rev" "${paths[@]}" | tar -x -C web/upstream
else
  curl -fsSL "https://codeload.github.com/lesomnus/payday/tar.gz/$rev" |
    tar -xz -C web/upstream --strip-components=1 "${paths[@]/#/payday-$rev/}"
fi
