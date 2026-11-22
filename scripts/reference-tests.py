"""Check the per-case ledger against the pinned reference and an optional Go run."""

import argparse
import csv
import json
import re
import subprocess
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REVISION = "53acc135cc084adafda524e882f19275c0d0cc96"
PACKAGE = "github.com/lesomnus/payday/internal/apptest/cmd"
DECLARATION = re.compile(r"^func\s+(Test\w+)\s*\(\s*\w+\s+\*testing\.T\s*\)")
ENVIRONMENTS = {
    "unit", "cli/native", "native/rekor-v1+v2", "native/witness-store+log", "store/sqlite", "store/postgres", "store/sqlite+postgres",
    "connect/sqlite", "connect/sqlite+postgres", "connect/postgres-notify", "connect/postgres+nats",
    "connect/tls-sqlite", "connect/cookies-sqlite", "connect/grant-source",
    "connect/sqlite+postgres;drpc/opfs",
}


def inventory(checkout):
    revision = subprocess.check_output(["git", "-C", checkout, "rev-parse", "HEAD"], text=True).strip()
    if revision != REVISION:
        raise ValueError(f"expected Payday {REVISION}, found {revision}")
    found = {}
    for path in sorted((checkout / "internal/apptest/cmd").glob("*_test.go")):
        for line, decl in enumerate(path.read_text().splitlines(), start=1):
            if match := DECLARATION.match(decl):
                name = match[1]
                if name in found:
                    raise ValueError(f"duplicate Go test {name}")
                found[name] = (path.name, str(line))
    if not found:
        raise ValueError("no Go reference tests found")
    return found


def check_run(path, names):
    outcomes = {}
    success = False
    with path.open() as stream:
        for line in stream:
            event = json.loads(line)
            if event.get("Package") != PACKAGE:
                continue
            result = event.get("Action")
            name = event.get("Test")
            if not name and result in ("pass", "fail"):
                success = result == "pass"
            if name and "/" not in name and result in ("pass", "fail", "skip"):
                if name in outcomes:
                    raise ValueError(f"duplicate result for {name}, use go test -count=1")
                outcomes[name] = result
    if not success or outcomes != dict.fromkeys(names, "pass"):
        missing = sorted(set(names) - outcomes.keys())
        extra = sorted(outcomes.keys() - names)
        incomplete = {k: v for k, v in outcomes.items() if v != "pass"}
        raise ValueError(f"incomplete reference run, missing={missing} extra={extra} outcomes={incomplete}")


def validate(row):
    case = row["go_test"]
    proof = row["evidence"]
    status = row["status"]
    issue = row["issue"]
    if status not in ("verified", "tracked"):
        raise ValueError(f"{case} has unknown status {status}")
    if status == "verified" and (issue != "-" or not proof):
        raise ValueError(f"{case} needs executable evidence")
    if status == "tracked" and not re.fullmatch(r"[1-9][0-9]*", issue):
        raise ValueError(f"{case} needs a follow-up issue")
    if bool(proof) != bool(row["environment"]):
        raise ValueError(f"{case} must pair its evidence with an environment")
    if not proof:
        return
    if row["environment"] not in ENVIRONMENTS:
        raise ValueError(f"{case} has unknown environment {row['environment']}")
    path, anchor, line = proof.partition("#L")
    if not anchor or not line.isdecimal() or not (ROOT / path).resolve().is_relative_to(ROOT):
        raise ValueError(f"{case} has invalid evidence {proof}")
    source = (ROOT / path).read_text().splitlines()
    if not 1 <= int(line) <= len(source) or not source[int(line) - 1].strip():
        raise ValueError(f"{case} has missing evidence at {proof}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkout", type=Path, help="Go Payday checkout at the pinned revision")
    parser.add_argument("--go-json", type=Path, help="complete go test -json ./cmd -count=1 output")
    args = parser.parse_args()
    expected = inventory(args.checkout)
    with (ROOT / "docs/reference-tests.tsv").open(newline="") as table:
        rows = list(csv.DictReader(table, delimiter="\t"))
    actual = {r["go_test"]: (r["file"], r["line"]) for r in rows}
    if len(actual) != len(rows):
        raise ValueError("duplicate ledger entries")
    if actual != expected:
        added = expected.keys() - actual.keys()
        removed = actual.keys() - expected.keys()
        moved = {n for n in expected.keys() & actual.keys() if expected[n] != actual[n]}
        raise ValueError(f"reference drift: added={sorted(added)} removed={sorted(removed)} moved={sorted(moved)}")
    for row in rows:
        validate(row)
    if args.go_json:
        check_run(args.go_json, set(actual))
    print(f"{len(rows)} Go cases: {dict(Counter(r['status'] for r in rows))}")
    if args.go_json:
        print(f"All {len(rows)} Go cases ran and passed.")


if __name__ == "__main__":
    main()
