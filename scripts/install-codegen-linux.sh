#!/usr/bin/env bash
# Install the pinned code generators for the Linux x86_64 CI runner.
set -euo pipefail
dest=${1:?provide an installation directory}
mkdir -p "$dest"
dest=$(cd "$dest" && pwd)
download() {
  local tool=$1 repo=$2 version=$3 digest=$4
  curl -fsSL --retry 3 "https://github.com/$repo/releases/download/v$version/$tool-v$version-linux-x86_64" -o "$dest/$tool"
  echo "$digest  $dest/$tool" | sha256sum -c -
  chmod +x "$dest/$tool"
}
download protoc-gen-buffa anthropics/buffa 0.8.1 52c227091a30cd73b04909f52a72c452e84fe5db3e6c612e93d062d274c90c78
download protoc-gen-buffa-packaging anthropics/buffa 0.8.1 e7f1fbddbd3ac413679bc6d7c7ad830c32601f2454350967b1b63dad0e2df8d2
download protoc-gen-connect-rust connectrpc/connect-rust 0.8.2 8990729f7a881026d4bbe04be0aea8710a01488af2659361bcca6728a65cb1d9
GOBIN="$dest" go install github.com/protobuf-orm/protoc-gen-orm-service@v0.0.0-20261121165649-aa6870b89525
GOBIN="$dest" go install github.com/protobuf-orm/protoc-gen-orm-rust@0a44aae475a576ba1e7019471e889306e60fc0fe
