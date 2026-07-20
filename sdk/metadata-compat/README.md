# Partial compat metadata

`sdk/metadata.scale` tracks the workspace runtime and is the canonical codegen
input for the Rust SDK. Live chains lag it; when an item the SDK uses changes
shape incompatibly, add a partial snapshot here scoped to the affected pallet,
generated from the oldest supported chain with that shape:

```bash
subxt metadata --url wss://<chain-rpc> --pallets <Pallet> -f bytes \
    > sdk/metadata-compat/<pallet>-v<spec_version>.scale
```

Then in `sdk/rust/src/compat.rs`: add a `#[subxt::subxt]` module for the
snapshot, a registry row (the key is derived from the file itself — see
`renew_registry()`), and a match arm at the call site. Dispatch hashes the
connected chain's item (subxt's per-item type-tree hash) and looks it up;
unknown shapes fail closed — deliberately strict: even a wire-compatible
evolution of the item's type tree needs a new row.

The TypeScript SDK (`sdk/typescript/src/compat.ts`) mirrors the registry with
PAPI's per-item checksums: pin the snapshot's checksum as a registry row and
add the encoder arm; at runtime the connected chain's metadata is checksummed
and looked up. Unit tests on both sides re-derive every key/checksum from the
committed files here, so a snapshot cannot drift from its registry rows.

Rules:

- Trimmed snapshots are safe only for **pallet-local** items: per-item
  validation hashes cover just the item's own type tree. Never encode
  `RuntimeCall`-embedding calls (`Sudo.sudo`, `Utility.batch_all`) from one —
  the reduced call enum cannot hash-match a live chain.
- After regenerating any `.scale`, run `cargo clean -p bulletin-sdk-rust`:
  cargo does not track the subxt macro's file input, so a stale expansion
  survives an ordinary rebuild.
- Delete a snapshot (and its module + dispatch arm) once no supported chain
  needs it.

## Snapshots

| File | Source chain (fetched) | Covers |
|---|---|---|
| `transaction-storage-v1000011.scale` | bulletin-westend v1000011, `wss://westend-bulletin-rpc.polkadot.io` (2026-07-08) | positional `renew(block, index)` |
