# Trail integrity

Set `PAYDIRT_AUDIT_KEY` to a hex HMAC key of at least 32 bytes. Native writes store a Payday v3 digest in the audit row and a v1 RowSeal for kinds with personal fields. The digest covers secret fields, and the seal also covers personal fields. Read both with `APP trail-row app.Holder UUID`. The Rust `Keys` API accepts previous verification keys and retired fingerprints.

Supply a PKCS#8 DER Ed25519 key file in `PAYDIRT_SIGNING_KEY` and an independently protected directory in `PAYDIRT_WITNESS_DIR`. Set the archive directory and encryption key as for `APP trail`. Run `APP trail-check checkpoint` after a maintenance pass. Run `APP trail-check verify` to authenticate every manifest row, decrypt its object and compare the signed chain to the witness. A checkpoint retries missing witness receipts. Put the witness under separate credentials or on an independently administered filesystem.

The pinned Go fixture uses upstream Payday to emit row MACs, seals, a signed manifest and two linked gzip checkpoints. Regenerate it with `PAYDAY_CHECKOUT=/path/to/payday scripts/trail-vectors.sh`. The Rust tests verify the Go output and detect modified rows, signatures and missing predecessor objects.

| Provider | Implementation and verification |
| --- | --- |
| Local file key | Native Ed25519 signer. The verifier accepts trusted PKIX Ed25519, ECDSA and RSA keys. Ed25519 is checked against Go. |
| AWS KMS and Vault Transit | A Rust signer or separate signing process can keep the key with the provider. Dedicated AWS SDK and Vault HTTP clients and live-provider verification remain open. |
| PKCS#11 and TPM | The same signer API supports deployment-managed devices. Direct module and TPM device drivers require separate integrations and hardware verification. |
| Directory witness | Publishes immutable numbered receipts outside the primary archive. Requires atomic hard links and directory sync. |
| Log | Publishes the checkpoint number and digest to stderr and records a local receipt. |
| Rekor v1 and v2 | Submits the digest of the checkpoint statement, proves inclusion in the configured log’s signed Merkle tree, and checks consistency against its current signed head. V1 also searches the deployment’s keys for checkpoints missing from the archive. |
| Other remote object stores | A deployment can implement the Rust Witness trait. |

For a process signer set `PAYDIRT_SIGNER_PROGRAM`, a JSON argument vector in `PAYDIRT_SIGNER_ARGS` and a PKIX DER public key in `PAYDIRT_SIGNER_PUBLIC_KEY`. The process reads the statement on stdin and returns its raw signature on stdout. The framework verifies each signature before publishing. `PAYDIRT_TRUST_KEYS` accepts a JSON list of trusted DER public key paths for rotation.

For a record shared by several tenants, the online window and final destruction date each follow the longest applicable window. Any owner’s hold protects the record. Both owners can export the same shared archive entry. Purging the filing tenant blanks the shared content and keeps the event. A host can queue a subject erasure using `Transaction::owe_trail_erasure` alongside its database update. The obligation becomes visible to maintenance on commit.

Run `APP trail-scan app.Cell` to check current rows and the latest recorded digest of each object, including a recorded object that disappeared. Reports include the number checked, heads found in the archive, skipped-kind reasons and per-object findings. `APP trail-record app.Cell '{"ids":["UUID"],"action":"/ops/Reviewed"}'` accepts the current state as a distinct signed act. Use `"erased":true` for an absent object with a prior audit line. A host may call `Transaction::record_objects` inside its business transaction.

Configure the archive directory to include it in `trail-scan` and `trail-row`. New signed objects bind the digest evidence and the hash of the encrypted file to the manifest signature. They can be checked with the configured signing identity and witness while the archive decryption key is withheld. Checking older objects without signed evidence requires their archive key.


Set `PAYDIRT_AUDIT_PREVIOUS_KEYS` to a JSON list of hexadecimal verification keys after rotation. `PAYDIRT_AUDIT_RETIRED` accepts released eight-byte fingerprints in the same format. `PAYDIRT_AUDIT_SINCE` marks the RFC 3339 start of digesting. A scan rejects a start time later than an authenticated digest. Reports distinguish retired keys, unavailable keys, forgotten seals and rows written before sealing started.

When maintenance destroys the last evidence of a live row, it carries the authenticated digest into a new audit entry before deleting the old evidence. A carry preserves the evidence of the original state, including a forgotten seal. A subsequent business write records the new state.


`PAYDIRT_WITNESSES` accepts a JSON list. Entries may be `log:`, `store:/path/to/independent-store`, or `rekor:https://log.example?key=/etc/paydirt/log.pem&v=2`. Supply the log's PEM or DER public key and optionally its signed-note origin. The local Ed25519 checkpoint signer signs a SHA-512 digest using Ed25519ph for Rekor. Entry bodies contain the digest, signature and deployment public key. The archive keeps the inclusion receipt for each checkpoint. V2 uses authenticated tiles to verify tree consistency. Detecting a v2 checkpoint removed with all its receipts requires an independent witness or log monitor.

`python3 scripts/test-rekor.py /path/to/payday` exercises both upstream log implementations, checks a tile boundary, and covers receipt corruption, a rewritten log, temporary unavailability and v1 truncation.


Host transactions can read a page with `Transaction::trail_page`. Pass the last row’s `Cursor` to resume in creation-time and identifier order across Audit and Access. A `Selection` filters by filing tenant, sole owner, co-owner, kind or object. `trail_tenants` seeks to successive filing tenants, and `owed_page` uses the same cursor order for the application's erasure queue. Existing pending obligations are upgraded to the schema's Owed type during maintenance; their first recorded queue timestamp is the migration time.

Each archived object has a signed row in the application's `Archived` table. A zero-ID row tracks the database checkpoint. Maintenance commits its intended membership before external publication and can finish a failed witness delivery after restart. Hosts can create an intent with `Transaction::archive_intent`, inspect the typed rows with `archive_records`, or sign a transition with `change_archive_record`. A transition checks the previously observed state and values before replacing the row.

After restoring an earlier backup, run `APP trail-check review`. The response gives the database and external checkpoint numbers, the review digest and the difference in membership. To accept a reviewed restore, run `APP trail-check '{"of":"sha256:…","restored":1,"by":"operator","why":"restored backup"}'` using the returned digest and the database’s actual checkpoint. Acceptance verifies the independently witnessed chain, reads and re-adopts missing objects from the signed checkpoint, and signs the decision into the next checkpoint. Changed findings require a fresh review.
