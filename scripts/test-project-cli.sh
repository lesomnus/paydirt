#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
bin=${PAYDIRT_BIN:-$repo/target/debug/paydirt}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
"$bin" new clinic "$work/app"
cd "$work/app"
"$bin" entity add --global Invoice
"$bin" entity list | rg 'app.Invoice[[:space:]]+global'
"$bin" gen
"$bin" buf --version | rg '^[0-9]+\.'
"$bin" buf --directory . ls-files | rg 'app/invoice.proto'
mkdir ts
cat >ts/package.json <<'JSON'
{"name":"clinic-page","dependencies":{"@bufbuild/protobuf":"^2.3.0","special":"1.2.3"}}
JSON
python3 - <<'PY'
from pathlib import Path
import re
p=Path('Cargo.toml')
s=re.sub(r'(github.com/lesomnus/paydirt",rev=")[0-9a-f]+',lambda m:m[1]+'a'*40,p.read_text())
p.write_text(s+'\n# application-owned manifest comment\n')
PY
"$bin" versions | rg '@bufbuild/protobuf.*2.12.1'
sha256sum src/*.rs >"$work/sources"
sha256sum Cargo.toml ts/package.json >"$work/manifest"
"$bin" upgrade --dry-run | rg 'update.*Cargo.toml'
sha256sum -c "$work/manifest"
"$bin" upgrade
rg -q 'application-owned manifest comment' Cargo.toml
python3 - <<'PY'
import json
v=json.load(open('ts/package.json'))['dependencies']
assert v=={'@bufbuild/protobuf':'^2.16.0','special':'1.2.3'}, v
PY
sha256sum -c "$work/sources"
"$bin" upgrade --dry-run | rg 'current'
# A removed option and an unwired drainer are distinct from ordinary generation drift.
echo 'message Old { string note = 1 [(payday.field) = {impersonal: true}]; }' >>proto/app/invoice.proto
if "$bin" doctor >"$work/diag" 2>&1; then exit 1; fi
rg -q 'obsolete declaration.*personal: false' "$work/diag"
sed -i '$d' proto/app/invoice.proto
python3 - <<'PY'
import json
p='app.json';v=json.load(open(p));v['watch']={'broker':'none','outbox':True,'drain_every_ms':5};open(p,'w').write(json.dumps(v))
PY
cat >>proto/app/invoice.proto <<'PROTO'
message Visit {
    bytes id = 1 [(orm.field) = {type:TYPE_UUID key:true default:""}];
    string purpose = 8;
    string visitor = 9 [(payday.field) = {personal:true}];
    google.protobuf.Timestamp date_created = 15 [(orm.field)={immutable:true default:""}];
    option (orm.message)={rpc:{crud:true}};
    option (payday.entity)={domain:201 global:{} erase:{hard:{}}};
}
PROTO
"$bin" gen
if "$bin" doctor >"$work/diag" 2>&1; then exit 1; fi
rg -q 'schema option: app.Visit.purpose' "$work/diag"
rg -q 'wiring: draining requires' "$work/diag"
