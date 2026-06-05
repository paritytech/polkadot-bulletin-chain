# Architecture

Repository layout, pallets, and runtimes.

## Repository layout

```
polkadot-bulletin-chain/
├── runtimes/
│   ├── bulletin-westend/              # Parachain runtime (Westend testnet)
│   │   └── integration-tests/         # XCM emulator integration tests
│   └── bulletin-paseo/                # Parachain runtime (Paseo testnet)
├── pallets/
│   ├── transaction-storage/           # Core storage pallet
│   │   └── primitives/                # Shared types (ContentHash, CID utilities)
│   ├── hop-promotion/                 # HOP pool data promotion to chain storage
│   └── common/                        # Shared pallet utilities (NoCurrency, call inspection)
├── sdk/
│   ├── rust/                          # Rust SDK (no_std compatible)
│   └── typescript/                    # TypeScript SDK (@parity/bulletin-sdk)
├── console-ui/                        # React web interface
├── examples/                          # JavaScript/TypeScript/Rust integration examples
├── stress-test/                       # Write throughput & Bitswap read benchmarks
├── docs/                              # SDK book, authorization docs, operational playbook
├── scripts/                           # Build, benchmarking, and deployment scripts
└── zombienet/                         # Local parachain network configurations
```

## Pallets

### pallet-transaction-storage

Core storage pallet providing distributed data storage and retrieval with authorization-based access control.

**Extrinsics:**
- `store` / `store_with_cid_config` - Store data (with optional CID codec/hash configuration)
- `renew` / `force_renew` - Extend retention of stored data (scheduled vs. immediate)
- `authorize_account` - Grant an account permission to store (with transaction/byte limits)
- `authorize_preimage` - Authorize storage of data with a specific content hash
- `refresh_account_authorization` / `refresh_preimage_authorization` - Extend authorization expiration

**Key features:**
- Authorization-based access control (account-scoped or content-addressed)
- Configurable retention period with automatic cleanup
- Auto-renewal tracking for important data
- Merkle-based storage proofs with chunk validation
- Soft-cap (priority signal) and hard-cap (per-window renewal quota) for storage capacity
- Feeless transaction support via `pallet-skip-feeless-payment`

### pallet-hop-promotion

Promotes near-expiry HOP (Hand-off Protocol) pool data to permanent chain storage. Uses general (unsigned authorized) transactions to fill unused blockspace without charging users. Validates sr25519 signatures and checks that the promoting account has an active Bulletin authorization.

### pallet-common

Shared utilities including `NoCurrency` (a no-op fungible currency for pallets that require one) and call inspection helpers for unwrapping utility/sudo/proxy wrappers during authorization tracking.

## Runtimes

Two parachain runtimes (`bulletin-westend`, `bulletin-paseo`) share the same pallet composition with network-specific constants. Both use 24-second slots (4 relay chain slots), 10 MiB max block length, and a ~14 day retention period.
</content>
