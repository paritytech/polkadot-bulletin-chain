# pallet-bulletin-data-renewal

> [!WARNING]
> This is a reference implementation provided for research, experimentation, and developer education. This code has not been fully audited. It is actively under development and may contain bugs, vulnerabilities, or incomplete features. It is not recommended for production use without independent review. Use at your own risk.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../../LICENSE-APACHE)
[![Status: experimental](https://img.shields.io/badge/status-experimental-yellow.svg)](#)

> Part of the [Polkadot Bulletin Chain](https://github.com/paritytech/polkadot-bulletin-chain).

Extends the retention of data stored in `pallet-bulletin-transaction-storage`, manually or automatically.

## Overview

Stored data is removed once its `RetentionPeriod` elapses. This pallet is the renewal layer on top of the storage pallet, which has no renewal vocabulary of its own: it exposes two opaque payloads (`EntryMeta`, `AuthorizationExtra`) that the runtime wires to this pallet's `EntryKind` and `PermanentExtent`.

Dispatchables:
- `renew(entry)` — register a one-shot renewal for an entry, identified by `Position { block, index }` or `ContentHash(hash)`
- `enable_auto_renew(content_hash)` / `disable_auto_renew(content_hash)` — register or cancel recurring renewal
- `force_renew(entry)` — renew synchronously, during block execution
- `process_pending_renewals` — mandatory inherent that drains the current block's renewal queue

Renewals fire at the retention boundary: the storage pallet's `handle_obsolete` hook queues registered entries into `PendingRenewals`, and the inherent renews them in the same block.

`renew` and `enable_auto_renew` are feeless. The transaction extension charges one transaction slot plus `size` bytes at registration, which prepays the first cycle; recurring registrations are charged per cycle thereafter.

Renewed bytes are capped twice — per account against the authorization's `bytes_allowance`, and chain-wide by `MaxPermanentStorageSize`. The two count differently: the per-account cap charges every renewal, while the chain-wide counter counts a content hash once however many overlapping renewals it has, so it tracks bytes on disk rather than references to them. Crossing 80% of the chain-wide cap emits `PermanentStorageNearCap` once per crossing, as a signal for governance to raise the cap.

## Dependencies

- [`pallet-bulletin-transaction-storage`](../transaction-storage/) — the storage pallet this layer renews entries in
- [`bulletin-transaction-storage-primitives`](../transaction-storage/primitives/) — CID utilities and shared types

## Security

See the [root README](../../README.md#security) for security notices and responsible deployment guidance.

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

Apache-2.0
