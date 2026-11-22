"""Compare all app descriptors generated from upstream's authored sources with its Go fixture."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

repo = Path(__file__).resolve().parents[1]
upstream = repo / "web/upstream"
cli = Path(os.environ.get("PAYDIRT_BIN", repo / "target/debug/paydirt")).resolve()


def descriptors(project, output):
    subprocess.run(["buf", "build", "--as-file-descriptor-set", "--exclude-source-info", "-o", output],
                   cwd=project, check=True)
    return {f["name"]: f for f in json.loads(output.read_bytes())["file"]
            if f["name"].startswith(("app/", "shared/"))}


with tempfile.TemporaryDirectory() as tmp:
    app = Path(tmp) / "app"
    app.mkdir()
    for name in ("buf.yaml", "buf.lock", "paydirt.schema.json"):
        shutil.copy(repo / name, app / name)
    source = upstream / "internal/apptest/proto"
    for path in source.rglob("*.proto"):
        rel = path.relative_to(source)
        if rel.parts[:2] == ("app", "payday") or path.name.endswith("_svc.g.proto"):
            continue
        dest = app / "proto" / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(path, dest)
    subprocess.run([cli, "gen", app], check=True)
    subprocess.run([cli, "gen", "--check", app], check=True)
    overlay = app / "proto/ext/app/robot_svc.ext.proto"
    original = overlay.read_bytes()
    signature = b"service RobotService {"
    assert original.count(signature) == 1
    overlay.write_bytes(original.replace(signature, signature + b"\n  rpc Get(RobotGetRequest) returns (Robot);"))
    conflict = subprocess.run([cli, "gen", app], capture_output=True, text=True)
    assert conflict.returncode and "redefines generated RPC Get" in conflict.stderr, conflict.stderr
    overlay.write_bytes(original)
    subprocess.run([cli, "gen", "--check", app], check=True)
    shutil.copytree(app / "generated/proto", app / "proto", dirs_exist_ok=True)
    actual = descriptors(app, Path(tmp) / "rust.json")
    expected = descriptors(upstream, Path(tmp) / "go.json")
    assert actual.keys() == expected.keys(), actual.keys() ^ expected.keys()
    assert len(actual) == 44, len(actual)
    for name in expected:
        assert actual[name] == expected[name], f"descriptor differs for {name}"
    print(f"Matched all {len(expected)} Payday app descriptors, including field presence, options and streaming.")
