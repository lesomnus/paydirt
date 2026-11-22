#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
bin=${PAYDIRT_BIN:-$repo/target/debug/paydirt}
work=$(mktemp -d)
server=
cleanup() {
  if [[ -n "$server" ]]; then kill "$server" 2>/dev/null || true; wait "$server" 2>/dev/null || true; fi
  rm -rf "$work"
}
trap cleanup EXIT
cd "$work"
mkdir app
(cd app && "$bin" new sample-app .)
cd app
if "$bin" new overwrite .; then exit 1; fi
printf '\n// user implementation\n' >> src/core.rs
"$bin" gen
"$bin" gen --check
grep -q 'user implementation' src/core.rs
cargo build --quiet
target=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
PAYDIRT_LISTEN="127.0.0.1:$port" "$target/debug/sample-app" &
server=$!
export PAYDIRT_URL="http://127.0.0.1:$port"
for n in {1..100}; do
  if curl -s -o /dev/null "$PAYDIRT_URL"; then break; fi
  sleep 0.1
done
test "$(node client.mjs)" = 'sample-app'
if node client.mjs missing; then exit 1; fi
cd ..
if "$bin" new '../outside'; then exit 1; fi
test ! -e outside
