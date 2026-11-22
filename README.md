# Paydirt

Paydirt is a Rust port of the Payday application framework. Declare an entity in protobuf, generate its service and Diesel storage, and implement your domain RPCs as Rust service layers. The same generated app can serve Connect over HTTP with PostgreSQL or SQLite, or run inside a web page with a separate SQLite Worker. The compatibility baseline is [Go Payday at `53acc135`](https://github.com/lesomnus/payday/tree/53acc135cc084adafda524e882f19275c0d0cc96).

## Create an app

Install Rust, Go and Buf 1.72. On Linux x86_64, run these commands from this repository to install the CLI and pinned protobuf generators.

```sh
cargo install --locked --path cli
scripts/install-codegen-linux.sh "$HOME/.local/bin"
export PATH="$HOME/.local/bin:$PATH"
paydirt new inventory
cd inventory
paydirt entity add --tenanted --watch Gauge .
paydirt gen
paydirt doctor
cargo run
```

In another terminal in the app directory, `node client.mjs` reads the deployment's public Setting over Connect. The native server uses `paydirt.sqlite3` by default. Set `DATABASE_URL` to a PostgreSQL URL to use PostgreSQL. `cargo run -- gauge --help` shows the generated operator commands. Remote calls use `PAYDIRT_URL` and `PAYDIRT_TOKEN`. The local provisioning command `cargo run -- app.Tenant Add '{"alias":"acme"}'` uses the binary's database connection. Configure the native token map with `PAYDIRT_PRINCIPALS`. The app README describes the token and ID format.

| App developer writes | Paydirt generates |
| --- | --- |
| Entity fields, keys, relationships and Payday options in `proto/` | Standard `*_svc.g.proto` contracts, protobuf messages and typed Connect services |
| Custom RPC declarations in `proto/ext/` | A merged service that includes both standard and custom methods |
| Business logic in `src/core.rs` or an app module and its registration in `src/lib.rs` | Delegating service layers, default Diesel sinks, relational models, migration metadata and operator commands |

`paydirt gen` replaces the framework-owned `generated/` tree. `paydirt gen --check` detects stale output. Requests pass through Gate, Audit, app Core and Sink. Core can use the typed next service to make scoped reads and writes in the original call's transaction. The Fleet acceptance fixture implements Robot.Move this way. [Robot Core](tests/fleet/robot.rs) · [generation and upgrade checks](scripts/test-generated-cell.sh)

## What is tested

- **Schema and storage.** CI compares all 44 application and shared file descriptors with the Go reference. A new project receives Cell CRUD from its schema alone. Database tests cover guards and collection updates, alias uniqueness, keyset pages, watch cancellation, version checks and migration of an existing database.
- **Application layers.** The same TypeScript clients run against the Go reference and the generated Rust service. The tests exercise Robot.Move, roles, tenant and group scope, account selection and revocation. A deliberate conflict on the second write in a custom Core verifies rollback of both rows, the audit trail and the outbox.
- **Optional services.** Fleet enables event and fact history, notification inboxes and read markers, usage meters and entitlement grants. The linked matrix records the upstream cases and Rust tests for each. [Compatibility matrix](https://github.com/lesomnus/paydirt/issues/16)
- **SDK.** CI runs Payday's original query, writes and store tests against both native implementations. It also runs the original sandbox suite against both implementations in Chromium. The browser persistence test closes and recreates both Rust Workers, then reads the saved data by ID. [SDK runner](scripts/test-sdk.sh) · [Worker persistence test](web/generated-browser.mjs)

The root `robot/` crate is the original compatibility fixture for Go's `internal/apptest`. It has its own handwritten handlers and smaller API. The CLI scaffold and generated Fleet tests above demonstrate the application development workflow.

## Browser sandbox

The page creates two dedicated Workers and transfers the ends of a private `MessageChannel` to them.

```text
Payday TypeScript Connect client
  → SandboxLink → createDrpcTransport → MessagePort
  → Rust app Worker (drpc-rs + Connect)
  → MessagePort → Rust storage Worker (Diesel + SQLite) → OPFS
```

Inside an app, run `paydirt sandbox init`, then `npm ci`, `npm run build:wasm` and `npm run dev` in `sandbox/`. Building requires the `wasm32-unknown-unknown` Rust target, Clang, an LLVM archiver and `wasm-bindgen-cli` 0.2.130. A web client can pass the transport from `openSandbox(principals)` to its generated Connect client and close the link on shutdown.

The database Worker owns the SQLite connection and transaction boundaries. It uses OPFS sync access handles in a secure browser context with one active database Worker per origin for this fixture. The browser interop test imports Payday's upstream generated TypeScript service and `SandboxLink` at commit `53acc135` and the published `@lesomnus/grpc-dgram@0.1.0`. The Rust Worker uses its own startup code and `dialWorker`. Both test clients send binary protobuf.

## Run the original Robot fixture

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

The fixture runs the same suite against native HTTP and the browser MessagePort. It covers cross-tenant access, duplicate-key rollback, a subscription that follows a renamed robot through erasure, cancellation, and cursor pagination. For SQLite it also reloads the page, recreates both Workers, and reads the same row by ID. The native executable accepts `PAYDIRT_LISTEN`, `DATABASE_URL` and `PAYDIRT_PRINCIPALS` (a JSON map of bearer tokens to hex tenant IDs).

To regenerate this repository's fixture, run `paydirt gen` at the root. The TypeScript fixture and upstream contract inputs can also be extracted from a local Payday checkout with `PAYDAY_CHECKOUT=/path/to/payday scripts/upstream.sh`.

## Updating a stored schema

The generated app checks its stored ORM model and physical database layout at startup. Field numbers identify existing columns across protobuf field renames. New tables, nullable columns, supported default-valued columns and index changes run in a single transaction. A unique index that conflicts with existing data rolls back its migration. Removing a stored column, changing its key/type or adding a required field that needs a backfill reports a version or schema error. The upgrade suite exercises both dialects with rows written against the previous generated model.

### Computed read resources

A message with `(payday.resource)` generates a read-only Connect service. Implement its
service trait and return the provider from the matching method on `storage::Layers`.
For example, `app.ComputedReading` generates `fn app_computed_reading(&self) -> impl
services::app::ComputedReadingService`. The default provider returns `Unimplemented`.

The provider owns its data source, checks access on every read and subsequent Watch
snapshot, and chooses its pagination and revision semantics. List can return metadata
even for an empty page. Watch replaces the complete collection on each message. The
same provider is mounted by the native and sandbox routers. [Runnable example](tests/fleet/readings.rs)

### Watch brokers and the outbox

Generated native apps select their Watch broker in `app.json`. Add a `watch` entry
when upgrading an older project. A new project explicitly selects `memory`.
The browser sandbox also uses the in-process broker in its server Worker. Only a
successful transaction in the SQLite Worker returns change metadata to that broker.

| Setting | Deployment |
| --- | --- |
| `{"broker":"memory"}` | Subscribers and writers share one server process. |
| `{"broker":"postgres"}` | Replicas share a PostgreSQL database. A subscriber listens on `payday_watch`. Set `dsn` to select a different broker database. |
| `{"broker":"nats","dsn":"nats://localhost:4222","namespace":"fleet"}` | Core NATS routes by deployment, service, key and notification audience. All replicas of a deployment use the same namespace. Broker credentials need publish and subscribe within that namespace, including its two-token sync subjects used to confirm a subscription before sending the initial snapshot. |
| `{"broker":"none"}` | Live subscriptions return FailedPrecondition. |

Subscriptions are installed before the initial snapshot or notification Ready.
The server reads current authorized state after a signal. A full reader queue
or a broken broker stream tells the consumer to reconnect.

Set `outbox` to record mutations durably. Set `drain_every_ms` to a positive interval
to publish queued rows, oldest first, in batches of 128. Successful publications
are acknowledged in a separate SQL transaction. A restart between publication
and deletion can repeat an event. Rust applications can call `Store::drain_once`
with a custom `paydirt::watch::Broker`. Existing outbox rows without route
metadata are delivered to matching service and key subscriptions.

Run `paydirt entity list` to see names, domain numbers and scopes from the
current schema. `paydirt buf` forwards arguments in the app's Buf workspace
and respects `PAYDIRT_BUF`. `paydirt versions` reports the active toolchain and
framework-managed Cargo and page dependencies. Preview manifest changes with
`paydirt upgrade --dry-run` and apply them with `paydirt upgrade`. Run Cargo
and the page package manager to refresh their locks after a manifest upgrade.
`paydirt doctor` checks generated outputs, source declarations, app wiring and
buildable entry points. It also reports unresolved personal-field decisions.

### TypeScript clients and application metadata

Install `@bufbuild/protoc-gen-es` in the application or its `ts` directory and run
`paydirt gen --ts`. The web page imports message and service descriptors from
`generated/ts`. Pass a service descriptor to Connect's `createClient`. Import
`domains.ts` to register identifier names, `entities.ts` for the store's entity
descriptions, and `events.ts` for the registry used to decode Any payloads.
Subsequent `gen` and `gen --check` include TypeScript for that application.

Rust Core can use `transport::domains()` to resolve current or historical names,
and `transport::header(&row)` to read identity, tenant, labels and timestamps
from a typed message. A declared empty field is distinguishable with `has`.
Generated entity call traits provide `list_all(&ctx, &request)`. The stream
honors the input cursor and filters, follows empty pages with a continuation,
and returns one error on a failed List or repeated token. Dropping it stops traversal.

The `external-app` CI job runs the pinned Payday TypeScript suites through
`scripts/test-sdk.sh`. Each uses TypeScript generated by Paydirt from the
source app schema. The Go comparison also runs Payday's original sandbox page.

| Upstream file | Runtime and behavior exercised |
| --- | --- |
| client.test.ts | Native Go and Rust servers, cross-package service discovery, identity and wire errors |
| query.test.ts | Native Go and Rust servers, cached queries and sign-in sessions |
| writes.test.ts | Native Go and Rust servers, mutation reconciliation |
| sandbox.browser.test.ts | Chromium, Go sandbox and generated Rust server/database workers |
| entities.test.ts | Generated service and entity registries |
| events.test.ts | Generated event types and Any decoding |
| jsonschema.test.ts | JSON schemas from generated messages |
| store.test.ts | Local SDK store and generated relationships |
| idb.test.ts | IndexedDB store with fake-indexeddb |
| device.test.ts | Browser device state and storage |
| react.test.tsx | React subscriptions, jsdom and an in-process test transport |
| code.test.tsx | Editor bindings in jsdom |
| devtools.test.tsx | Devtools panel in jsdom with generated entity declarations |
