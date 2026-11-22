#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
app=$(mktemp -d)
trap 'rm -rf "$app"' EXIT
cp "$repo/buf.yaml" "$repo/buf.lock" "$app/"
mkdir -p "$app/proto/catalog"
file="$app/proto/catalog/part.proto"
cat > "$file" <<'PROTO'
edition = "2023";
package catalog;
import "orm.proto";
option go_package = "example.org/catalog";
option features.field_presence = IMPLICIT;
message Part {
  option (orm.message) = {rpc:{crud:true}};
  int32 id = 1 [(orm.field)={key:true}];
  string label = 2;
}
PROTO
bin=${PAYDIRT_BIN:-$repo/target/debug/paydirt}
"$bin" gen "$app"
cp "$app/generated/storage/model.json" "$app/v1.json"
sed -i 's/string label = 2;/string name = 2 [(orm.field)={unique:true}]; string note = 3 [(orm.field)={nullable:true}];/' "$file"
"$bin" gen "$app"
"$bin" gen --check "$app"
PAYDIRT_V1="$app/v1.json" PAYDIRT_V2="$app/generated/storage/model.json" cargo test --locked --manifest-path "$repo/Cargo.toml" -p paydirt-store --features postgres,sqlite --test upgrade
