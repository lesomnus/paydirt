# Payday reference app tests

[reference-tests.tsv](reference-tests.tsv) lists every top-level test in the Go reference app’s cmd package. The baseline is [Payday at 53acc135](https://github.com/lesomnus/payday/tree/53acc135cc084adafda524e882f19275c0d0cc96/internal/apptest/cmd). Each Go case is identified by name and its original file and line. Subtests remain part of their parent.

A verified row identifies a Rust-side fixture exercising the corresponding observable behavior. The environment column distinguishes direct store assertions, native Connect, CLI processes, and browser dRPC with SQLite on OPFS in a separate Worker. An evidence link in a tracked row identifies existing partial coverage. The linked issue owns the remaining assertion or implementation. The ledger counts cases, so a single Rust fixture may provide evidence for several rows. It does not count individual Go and Rust assertions or statements.

The reference workflow runs the original Go tests, including their PostgreSQL-only cases, and validates that every named case passed. Independently, the check and cli workflows exercise the Rust store, generated external application and Chromium. To reproduce from a clean pinned Go checkout with a disposable UTF-8 PostgreSQL database:

```bash
export PAYDAY_CHECKOUT=/path/to/payday
export PDTEST_POSTGRES='postgres://user:password@127.0.0.1/reference?sslmode=disable'
(cd "$PAYDAY_CHECKOUT/internal/apptest" && go test -json ./cmd -count=1) > /tmp/payday-reference.jsonl
python3 scripts/reference-tests.py "$PAYDAY_CHECKOUT" --go-json /tmp/payday-reference.jsonl
```

On a baseline update, change the checkout revision in the reference workflow and verifier, then run the verifier. It reports new, removed and relocated Go cases. Update the evidence and follow-up issue for each changed behavior.
