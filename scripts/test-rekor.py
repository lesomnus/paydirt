"""Run the Rust witness against both pinned upstream Rekor test servers."""
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

repo = Path(__file__).resolve().parents[1]
checkout = Path(sys.argv[1]).resolve()
build = subprocess.run(
    ["cargo", "test", "--locked", "-p", "paydirt-store", "--features", "sqlite",
     "--test", "rekor", "--no-run", "--message-format=json"],
    cwd=repo, check=True, stdout=subprocess.PIPE, text=True,
)
binary = next(msg["executable"] for line in build.stdout.splitlines()
              if (msg := json.loads(line)).get("executable") and msg["target"]["name"] == "rekor")
with tempfile.TemporaryDirectory() as tmp:
    overlay = Path(tmp) / "overlay.json"
    virtual = checkout / "trail/witnessrekor/rekortest/paydirt_test.go"
    overlay.write_text(json.dumps({"Replace": {str(virtual): str(repo / "scripts/fixtures/rekor_test.go")}}))
    subprocess.run(["go", "test", "-overlay", str(overlay), "-v", "./trail/witnessrekor/rekortest",
                    "-run", "^TestRustDeployment$", "-count=1"], cwd=checkout, check=True,
                   env={**os.environ, "PAYDIRT_REKOR_TEST_BIN": binary})
