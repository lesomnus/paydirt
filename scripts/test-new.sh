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
export DATABASE_URL="$work/sample.sqlite3"
mkdir app
(cd app && "$bin" new sample-app .)
cd app
if "$bin" new overwrite .; then exit 1; fi
cat > tests/extension.rs <<'RS'
use sample_app::{proto::app::{SettingGetRequest,Setting},transport::{Database,MODEL,POLICIES}};
use connectrpc::client::call_unary;
use serde_json::json;
#[tokio::test]
async fn user_core_describes_the_setting(){
    let fixture=paydirt_test::Fixture::new().unwrap();
    let db=Database::from_store(fixture.store(MODEL,POLICIES).unwrap(),"{}").unwrap();
    let (transport,config)=paydirt_test::client(sample_app::router(db,"app".into()));
    let config=config.with_default_header("x-deployment","west");
    let req:SettingGetRequest=serde_json::from_value(json!({"ref":{"alias":"service"}})).unwrap();
    let got=call_unary::<_,_,<Setting as buffa::HasMessageView>::View<'static>>(&transport,&config,"app.SettingService","Describe",req,Default::default()).await.unwrap();
    assert_eq!(got.into_owned().value,"west/service");
}
RS
cargo test --quiet --test core --test extension
cargo build --quiet
mkdir -p proto/ext/app
cat >proto/ext/app/setting_svc.ext.proto <<'PROTO'
edition = "2023";
package app;
import "app/setting.proto";
import "google/protobuf/empty.proto";
service SettingService {
  rpc Describe(SettingGetRequest) returns (Setting);
  rpc Ping(google.protobuf.Empty) returns (google.protobuf.Empty);
  rpc Pending(SettingGetRequest) returns (Setting);
  rpc Import(stream SettingGetRequest) returns (Setting);
  rpc Follow(stream SettingGetRequest) returns (stream Setting);
}
PROTO
# Only the new behavior belongs to Core. Get still resolves in the settings sink.
cat >src/core.rs <<'RS'
use crate::{proto::app::*, services::app::{SettingService, SettingServiceLayer}};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};

pub struct Core;

#[allow(refining_impl_trait)]
impl<N: SettingService> SettingServiceLayer<N> for Core {
    async fn describe(&self, _: &N, ctx: RequestContext, req: ServiceRequest<'_, SettingGetRequest>)
        -> ServiceResult<Setting> {
        let request = req.to_owned_message();
        let label = ctx.headers().get("x-deployment").and_then(|h| h.to_str().ok()).unwrap_or("");
        let Some(setting_ref::Key::Alias(alias)) = &request.r#ref.key else {
            return Err(connectrpc::ConnectError::invalid_argument("alias required"));
        };
        Response::ok(Setting { value: format!("{label}/{alias}"), ..Default::default() })
    }
}
RS
# The application explicitly exposes both new RPCs to unauthenticated clients.
sed -i 's|"/app.SettingService/Get"|"/app.SettingService/Get","/app.SettingService/Describe","/app.SettingService/Pending"|' src/lib.rs
cp src/core.rs ../original-core.rs
cp src/sink.rs ../original-sink.rs
"$bin" gen
"$bin" gen --check
cmp src/core.rs ../original-core.rs
cmp src/sink.rs ../original-sink.rs
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
node --input-type=module <<'JS'
import assert from "node:assert/strict";
async function call(method, alias = "service") {
  const result = await fetch(process.env.PAYDIRT_URL + "/app.SettingService/" + method, {
    method: "POST",
    headers: {"content-type":"application/json","connect-protocol-version":"1","x-deployment":"west"},
    body: JSON.stringify({ref: {alias}}),
  });
  return [result.status, await result.json()];
}
const [ok, custom] = await call("Describe");
assert.equal(ok, 200);
assert.equal(custom.value, "west/service");
const [absent, missing] = await call("Get", "missing");
assert.equal(absent, 404);
assert.equal(missing.code, "not_found");
const [error, pending] = await call("Pending");
assert.equal(error, 501);
assert.equal(pending.code, "unimplemented");
JS
"$bin" entity add --watch Gauge .
"$bin" entity add --global --file gauge.proto Station .
if "$bin" entity add Gauge .; then exit 1; fi
if "$bin" doctor .; then exit 1; fi
"$bin" gen .
"$bin" doctor .
cd ..
if "$bin" new '../outside'; then exit 1; fi
test ! -e outside
