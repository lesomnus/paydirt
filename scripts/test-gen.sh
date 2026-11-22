#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
bin=${PAYDIRT_BIN:-$repo/target/debug/paydirt}
app=$(mktemp -d)
trap 'rm -rf "$app"' EXIT
cp "$repo/buf.yaml" "$repo/buf.lock" "$app/"
mkdir -p "$app/proto/app" "$app/src"
cd "$app"
cat >proto/app/catalog.proto <<'PROTO'
edition = "2023";
package catalog.v1;
import "orm.proto";
option go_package = "example.org/catalog";
option features.field_presence = IMPLICIT;
message Part {
  int32 id = 1 [(orm.field) = {key: true}];
  string description = 4;
  option (orm.message) = {rpc: {crud: true}};
}
PROTO
refuses() {
  if "$@"; then echo "unexpected success: $*" >&2; exit 1; fi
}
refuses "$bin" gen --check
test ! -e generated
"$bin" gen
grep -q 'service PartService' generated/proto/app/catalog_svc.g.proto
grep -q 'message PartPatchRequest' generated/proto/app/catalog_svc.g.proto
grep -q 'pub fn by_id' generated/orm/app/catalog_svc.g.orm.rs
cp -r generated baseline
"$bin" gen
"$bin" gen --check
diff -r baseline generated
# A consumer compiles and uses the actual generated contract.
cat >Cargo.toml <<'MANIFEST'
[package]
name = "paydirt-generation-consumer"
version = "0.1.0"
edition = "2024"
[features]
client = ["connectrpc/client"]
[dependencies]
buffa = { version="=0.8.1", features=["json"] }
buffa-types = { version="=0.8.1", features=["json"] }
connectrpc = { version="=0.8.2", default-features=false, features=["json"] }
serde = {version="1", features=["derive"]}
serde_json = "1"
MANIFEST
cat >src/main.rs <<'RS'
#[path = "../generated/messages/mod.rs"] pub mod proto;
#[path = "../generated/services/mod.rs"] pub mod services;
mod orm { include!("../generated/orm/app/catalog_svc.g.orm.rs"); }
fn main() {
    use proto::catalog::v1::*;
    let req = PartRef::by_id(7).pick();
    assert_eq!(req.r#ref.key, Some(part_ref::Key::Id(7)));
}
RS
cargo run --quiet
echo '// app-owned customization' >>src/main.rs
sed -i 's/string description = 4;/string description = 4; string label = 5;/' proto/app/catalog.proto
refuses "$bin" gen --check
diff -r baseline generated
"$bin" gen
grep -q label generated/proto/app/catalog_svc.g.proto
rm generated/services/mod.rs
echo stale >generated/stale
refuses "$bin" gen --check
test -f generated/stale && test ! -e generated/services/mod.rs
"$bin" gen
test ! -f generated/stale && test -f generated/services/mod.rs
grep -q 'app-owned customization' src/main.rs
cp -r generated before_failure
echo invalid >>proto/app/catalog.proto
refuses "$bin" gen
diff -r before_failure generated
