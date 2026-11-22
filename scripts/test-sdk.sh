#!/usr/bin/env bash
# Run the pinned TypeScript files without changing their test cases or service descriptors.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
ref=${PAYDIRT_GO_CHECKOUT:?set PAYDIRT_GO_CHECKOUT to the pinned Payday clone}
app=${CELL_PROJECT:?set CELL_PROJECT to the generated Fleet application}
suite="$ref/internal/apptest/ts"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
if [[ "${PAYDIRT_SDK_DEPS_READY:-0}" != 1 ]]; then
  npm ci --prefix "$ref/ts" --no-audit --no-fund
  npm run build --prefix "$ref/ts"
  npm ci --prefix "$suite" --no-audit --no-fund
  if [[ "${PAYDIRT_TEST_WASM:-0}" == 1 ]]; then "$suite/node_modules/.bin/playwright" install chromium; fi
fi
cp -a "$suite"/{src,gen,package.json,vite.config.ts,index.html,sandbox.html} "$work/"
ln -s "$suite/node_modules" "$work/node_modules"
cat > "$work/vitest.config.ts" <<'CONFIG'
import {defineConfig} from "vitest/config";
export default defineConfig({resolve:{dedupe:["react","react-dom"]},test:{fileParallelism:false}});
CONFIG
if [[ "${PAYDIRT_SDK_ONLY_BROWSER:-0}" != 1 ]]; then
  if [[ -z "${PAYDIRT_SDK_SERVER:-}" ]]; then
    mkdir -p "$app/src/bin"
    cp "$repo/tests/fleet/sdk.rs" "$app/src/bin/sdk.rs"
    # Only the test issuer needs a localhost sign-in endpoint and a disposable database.
    if ! rg -q '^axum=' "$app/Cargo.toml"; then sed -i '/^\[target.*not(target_arch.*dependencies\]/a axum="0.8"\ntempfile="3"' "$app/Cargo.toml"; fi
    cargo build --manifest-path "$app/Cargo.toml" --bin sdk --quiet
    target=$(cargo metadata --manifest-path "$app/Cargo.toml" --no-deps --format-version 1 | python3 -c 'import sys,json;print(json.load(sys.stdin)["target_directory"])')
    PAYDIRT_SDK_SERVER="$target/debug/sdk"
  fi
  for binary in "${PAYDIRT_GO_REFERENCE:?}" "$PAYDIRT_SDK_SERVER"; do
    (cd "$work" && PDTEST_TESTSRV="$binary" "$suite/node_modules/.bin/vitest" run src/query.test.ts src/writes.test.ts src/store.test.ts)
  done
fi
if [[ "${PAYDIRT_TEST_WASM:-0}" == 1 ]]; then
  if [[ "${PAYDIRT_SDK_ONLY_BROWSER:-0}" != 1 ]]; then
    mkdir -p "$suite/public"
    (cd "$suite" && npm run wasm && PDTEST_SANDBOX=1 ./node_modules/.bin/vitest run src/sandbox.browser.test.ts)
  fi
  mkdir -p "$work/rust"
  cp "$app/sandbox"/{client.js,link.ts,modules.js} "$work/rust/"
  cp -a "$app/sandbox/public" "$work/public"
  cp "$repo/tests/fleet/sdk-sandbox.ts" "$work/src/sandbox.ts"
  (cd "$work" && PDTEST_SANDBOX=1 "$suite/node_modules/.bin/vitest" run src/sandbox.browser.test.ts)
fi

