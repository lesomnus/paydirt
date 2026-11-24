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
{"package":"app","go_package":"example.org/resources","entities":false}
JSON
cat >proto/app/reading.proto <<'PROTO'
edition = "2023";
package app;
import "payday.proto";
option go_package = "example.org/resources";
option features.field_presence = IMPLICIT;
message Reading {
  option (payday.resource) = {filter:".app.ReadingFilter"};
  bytes id = 1;
  string value = 2;
}
message ReadingFilter {}
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
assert json.loads(Path("generated/storage/model.json").read_text())["entities"] == []
assert not Path("generated/proto/app/payday/holder.proto").exists()
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
cat >src/lib.rs <<'RS'
#[path="../generated/messages/mod.rs"] pub mod proto;
#[path="../generated/services/mod.rs"] pub mod services;
pub mod storage { include!("../generated/storage/sinks.rs"); }
#[cfg(test)]
struct App;
struct Readings;
#[cfg(test)]
impl storage::Layers for App {
    fn app_reading(&self) -> impl services::app::ReadingService { Readings }
}
#[allow(refining_impl_trait)]
impl services::app::ReadingService for Readings {
    async fn get(&self, _: connectrpc::RequestContext, req: connectrpc::ServiceRequest<'_,proto::app::ReadingGetRequest>) -> connectrpc::ServiceResult<proto::app::Reading> {
        connectrpc::Response::ok(proto::app::Reading {id:req.to_owned_message().r#ref.id.clone(),value:"computed".into(),..Default::default()})
    }
    async fn list(&self, _: connectrpc::RequestContext, _: connectrpc::ServiceRequest<'_,proto::app::ReadingListRequest>) -> connectrpc::ServiceResult<proto::app::ReadingListResponse> {
        connectrpc::Response::ok(proto::app::ReadingListResponse::default())
    }
    async fn refresh(&self, ctx: connectrpc::RequestContext, req: connectrpc::ServiceRequest<'_,proto::app::ReadingGetRequest>) -> connectrpc::ServiceResult<proto::app::Reading> {
        self.get(ctx,req).await
    }
}
#[tokio::test]
async fn computed_services_mount_without_storage() {
    use tower::ServiceExt;
    use http_body_util::{BodyExt, Full};
    let router=storage::mount(connectrpc::Router::new(),&App);
    let rpc=connectrpc::ConnectRpcService::new(router);
    for method in ["Get","Refresh"] {
        let req=http::Request::builder().method("POST").uri(format!("/app.ReadingService/{method}"))
            .header("content-type","application/json").header("connect-protocol-version","1")
            .body(Full::new(bytes::Bytes::from_static(br#"{"ref":{"id":"AQ=="}}"#))).unwrap();
        let res=rpc.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(),200);
        let value:serde_json::Value=serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(value["id"],"AQ==");
        assert_eq!(value["value"],"computed");
    }
    struct Missing;
    impl storage::Layers for Missing {}
    let rpc=connectrpc::ConnectRpcService::new(storage::mount(connectrpc::Router::new(),&Missing));
    let req=http::Request::builder().method("POST").uri("/app.ReadingService/List")
        .header("content-type","application/json").header("connect-protocol-version","1")
        .body(Full::new(bytes::Bytes::from_static(b"{}"))).unwrap();
    let res=rpc.oneshot(req).await.unwrap();
    assert_eq!(res.status(),501);
}
RS
cargo test --quiet
cp -r generated before_failure
cat >proto/app/stored.proto <<'PROTO'
edition = "2023";
package app;
import "orm.proto";
import "payday.proto";
option go_package = "example.org/resources";
message Stored {
  option (orm.message) = {rpc:{get:{}}};
  option (payday.entity) = {domain:77,global:{},trail:{off:{why:"Read-only deployment configuration"}},erase:{hard:{}}};
  bytes id = 1 [(orm.field)={key:true,type:TYPE_UUID}];
}
PROTO
if "$bin" gen >failure.log 2>&1; then
  echo 'entities:false accepted an ORM entity' >&2
  exit 1
fi
# Payday rejects a stored entity without the required framework entities before
# the final storage-model check. Neither path may publish a partial result.
if ! grep -q 'nothing in this schema declares it' failure.log; then
  cat failure.log >&2
  exit 1
fi
diff -r before_failure generated
