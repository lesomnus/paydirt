# Paydirt

Paydirt is an early Rust port of [Payday](https://github.com/lesomnus/payday). The first application implements a subset of Payday's `app.RobotService`. The same Rust handlers serve native Connect RPC with PostgreSQL and browser dRPC with SQLite. Diesel implements the queries on both backends.

## Implemented

- `Add` mints a domain-7 Payday UUIDv8 or validates a supplied ID. `AddMany` writes the entire batch and its change records in one transaction.
- `Get` accepts an ID or a tenant-qualified alias. `List` supports tenant and robot filters and forward keyset pagination by creation time and ID. Page tokens belong to Paydirt.
- `Patch` changes an alias. `Erase` soft-deletes a row, releases its alias and reports whether that call erased it.
- `Watch` resolves up to 32 robot references under one database snapshot, then follows their IDs from the same tenant revision. It sends the initial state unless skipped, committed postimages on changes, and an ID with no value when a previously delivered row is erased. The example polls its transactional change table every 100 ms while idle.
- A host-provisioned bearer token identifies the caller's tenant. SQL queries scope reads and mutations to that tenant.

The root crate contains the reusable ID and tenant-token primitives. The robot application uses checked-in Buffa, Connect and [protoc-gen-orm-rust](https://github.com/protobuf-orm/protoc-gen-orm-rust) output. This original compatibility fixture has handwritten storage and RPC handlers. CLI-created apps use generated service sinks backed by the common Diesel store. Further work includes generating the entity write path and migrations, porting Payday's gate, audit and outbox layers, and supporting additional fields, orders, and methods such as `Apply` and `Move`.

## Generate Rust contracts for an application

Install the CLI with `cargo install --path cli --locked`. Generation uses Buf 1.72, Buffa 0.8.1 (including its packaging plugin), Connect Rust 0.8.2, and the ORM service and Rust plugins. On Linux x86_64, `scripts/install-codegen-linux.sh "$HOME/.local/bin"` installs the pinned plugins. Put that directory and `buf` on your path.

Create a standalone project with `paydirt new inventory` (or `paydirt new inventory .` in an empty directory). In the project run `cargo run`. Run `node client.mjs` in another terminal to query the service setting over Connect. The initial handler reads the deployment name from `app.json`. The app assembles its typed Core and settings sink in `src/lib.rs`. Adding a tenanted entity such as Cell and running `paydirt gen` registers its standard RPCs and generates its relational schema. The app binary accepts operator commands such as `cargo run -- app.Tenant Add '{"alias":"my-team"}' `. Its HTTP server resolves opaque tokens from `PAYDIRT_PRINCIPALS`. Implement individual methods on the generated `SettingServiceLayer` in `src/core.rs`. All other calls, including streams, pass to the next service with their request context. The bottom of the stack returns the Connect `unimplemented` status for an unhandled RPC.

Place the app's entity declarations in `proto/` with a v2 `buf.yaml` at the project root and pin the dependencies in `buf.lock`. Run `paydirt gen` there or pass the project directory. The CLI first generates standard ORM service contracts, then compiles messages, services and Rust lookup helpers from those contracts. It stages all outputs before replacing its owned `generated/` tree. Use `paydirt gen --check` in CI. A Payday app also has `paydirt.schema.json` naming the app's protobuf and Go package. The CLI copies the pinned framework entities into the app package, applies app options and entity overlays, runs Payday's proto plugin for List, Watch and computed resources, and merges service overlays from `proto/ext/`. The Rust runtime implements a subset of these generated contracts. CI compares all 44 app/shared file descriptors of upstream's reference app, including custom field options, with the output generated from the same source declarations.

## Browser sandbox

The page creates two dedicated Workers and transfers the ends of a private `MessageChannel` to them.

```text
Payday TypeScript RobotService client
  → SandboxLink → createDrpcTransport → MessagePort
  → Rust RobotService Worker (drpc-rs + Connect)
  → MessagePort → Rust storage Worker (Diesel + SQLite) → OPFS
```

The database Worker owns the SQLite connection and transaction boundaries. It uses OPFS sync access handles in a secure browser context with one active database Worker per origin for this fixture. The browser interop test imports Payday's upstream generated TypeScript service and `SandboxLink` at commit `53acc135` and the published `@lesomnus/grpc-dgram@0.1.0`. The Rust Worker uses its own startup code and `dialWorker`. Both test clients send binary protobuf.

## Run the interoperability test

Install Rust with `wasm32-unknown-unknown`, Clang and LLVM's archiver, the PostgreSQL client development library, Node 24+, and `wasm-bindgen-cli` 0.2.130. Set `DATABASE_URL` to a disposable PostgreSQL database. The test creates its tables and uses fresh tenant identities on each run.

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked wasm-bindgen-cli --version 0.2.130
cargo test --workspace --locked
cargo build --locked -p paydirt-robot --features native-server --bin robot-server
scripts/wasm.sh
scripts/upstream.sh
cd web
npm ci
npx playwright install chromium
npm test
```

The test runs the same suite against native HTTP and the browser MessagePort. It covers cross-tenant access, duplicate-key rollback, a subscription that follows a renamed robot through erasure, cancellation, and cursor pagination. For SQLite it also reloads the page, recreates both Workers, and reads the same row by ID. The native executable accepts `PAYDIRT_LISTEN`, `DATABASE_URL` and `PAYDIRT_PRINCIPALS` (a JSON map of bearer tokens to hex tenant IDs).

To regenerate this repository's fixture, run `paydirt gen` at the root. The TypeScript fixture and upstream contract inputs can also be extracted from a local Payday checkout with `PAYDAY_CHECKOUT=/path/to/payday scripts/upstream.sh`.

## Updating a stored schema

The generated app checks its stored ORM model and physical database layout at startup. Field numbers identify existing columns across protobuf field renames. New tables, nullable columns, supported default-valued columns and index changes run in a single transaction. A unique index that conflicts with existing data rolls back its migration. Removing a stored column, changing its key/type or adding a required field that needs a backfill reports a version or schema error. The upgrade suite exercises both dialects with rows written against the previous generated model.
