# pallet-bulletin-transaction-storage

Transaction storage pallet for the Polkadot Bulletin Chain. Indexes transactions and manages storage proofs.

## Overview

This pallet provides distributed data storage on-chain with proof-of-storage guarantees. It is designed for chains with no transaction fees and data is retrievable via the Bitswap protocol using content-addressed CIDs.

Key features:
- Store arbitrary data on-chain via the `store` extrinsic
- Automatic data removal after a configurable `RetentionPeriod` (default: 14 days at 6s block time)
- Data renewal to extend retention via `renew`
- Validators submit proofs of storing random data chunks when producing blocks
- CID generation for content-addressed data retrieval via Bitswap

## Usage

### Storing data

Use the `transactionStorage.store` extrinsic to store data. A CID is generated from the content hash for retrieval via Bitswap.

### Renewing data

To prevent data from being removed after the retention period, use `transactionStorage.renew(block, index)` where `block` is the block number of the previous store or renew transaction, and `index` is the index of that transaction in the block.

### Retrieving data

Stored data is retrievable via the Bitswap protocol using the CID generated at storage time.

## Dependencies

- [`bulletin-transaction-storage-primitives`](primitives/) — CID utilities and shared types
- `sp-transaction-storage-proof` — Storage proof verification from Polkadot SDK

License: Apache-2.0
