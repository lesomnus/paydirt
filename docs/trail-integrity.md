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
| Log, remote object store, Rekor | A deployment can implement the Rust Witness trait. Dedicated remote adapters and Rekor inclusion-proof verification remain open. |

For a process signer set `PAYDIRT_SIGNER_PROGRAM`, a JSON argument vector in `PAYDIRT_SIGNER_ARGS` and a PKIX DER public key in `PAYDIRT_SIGNER_PUBLIC_KEY`. The process reads the statement on stdin and returns its raw signature on stdout. The framework verifies each signature before publishing. `PAYDIRT_TRUST_KEYS` accepts a JSON list of trusted DER public key paths for rotation.
