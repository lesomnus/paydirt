#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
bin=${PAYDIRT_BIN:-$repo/target/debug/paydirt}
app=$(mktemp -d)
trap 'rm -rf "$app"' EXIT
cp "$repo/cli/templates/app/buf.yaml" "$repo/cli/templates/app/buf.lock" "$app/"
mkdir -p "$app/proto/app" "$app/proto/ext/app" "$app/src"
cd "$app"
cat >paydirt.schema.json <<'JSON'
{"package":"app","go_package":"example.org/resources"}
JSON
cat >proto/app/reading.proto <<'PROTO'
edition = "2023";
package app;
import "payday.proto";
import "orm.proto";
import "app/payday/tenant.proto";
option go_package = "example.org/resources";
option features.field_presence = IMPLICIT;
message Reading {
  option (orm.message) = {rpc:{get:{}}};
  option (payday.entity) = {domain:77,trail:{off:{why:"Ephemeral values supplied by the application layer"}},erase:{hard:{}}};
  bytes id = 1 [(orm.field)={key:true,type:TYPE_UUID}];
  Tenant tenant = 2 [(orm.edge)={immutable:true}];
  string value = 8;
}
message Projection {
  option (payday.resource) = {filter:".app.ProjectionFilter"};
  bytes id = 1;
  string value = 2;
}
message ProjectionFilter {}
PROTO
cat >proto/ext/app/reading_svc.ext.proto <<'PROTO'
edition = "2023";
package app;
import "app/reading.proto";
option go_package = "example.org/resources";
service ReadingService { rpc Refresh(ReadingGetRequest) returns (Reading); }
PROTO
"$bin" gen
"$bin" gen --check
python3 - <<'PY'
import json
from pathlib import Path
entities=json.loads(Path("generated/storage/model.json").read_text())["entities"]
assert any(e["name"] == "app.Reading" for e in entities)
assert Path("generated/proto/app/payday/holder.proto").exists()
assert Path("generated/server.rs").exists()
assert Path("generated/proto/app/reading_svc.g.proto").exists()
PY
# This consumer has no Database type, ORM runtime, SQL driver or schema store.
# Generated composition must compile and dispatch the same generated contract.
cat >Cargo.toml <<'TOML'
[package]
name = "paydirt-resource-consumer"
version = "0.1.0"
edition = "2024"
[features]
client = ["connectrpc/client"]
[dependencies]
buffa = {version="=0.8.1",features=["json"]}
buffa-types = {version="=0.8.1",features=["json"]}
connectrpc = {version="=0.8.2",default-features=false,features=["server","json"]}
serde = {version="1",features=["derive"]}
serde_json = "1"
http = "1"
http-body-util = "0.1"
bytes = "1"
tokio = {version="1",features=["macros","rt"]}
tower = {version="0.5",features=["util"]}
TOML
cp "$repo/tests/composition/lib.rs" src/lib.rs
cargo test --quiet
cp -r generated before_failure
# Storage is selected when composing the server, never by disabling entities.
python3 - <<'PYJSON'
import json
from pathlib import Path
p=Path("paydirt.schema.json")
v=json.loads(p.read_text())
v["entities"]=False
p.write_text(json.dumps(v))
PYJSON
if "$bin" gen >failure.log 2>&1; then
  echo 'obsolete entities switch was accepted' >&2
  exit 1
fi
grep -q 'unknown field `entities`' failure.log
diff -r before_failure generated
