# Polkadot Bulletin chain

The Bulletin chain is a specialized Polkadot parachain providing distributed
data storage and retrieval infrastructure for the Polkadot ecosystem. It is run
as a parachain using the Polkadot SDK's `polkadot-omni-node` binary against the
`bulletin-westend-runtime` WASM.

## Runtime functionality

The Bulletin chain runtime stores transactions for a configurable retention
period (currently set at 2 weeks) and provides proof of storage. It uses Aura
for parachain block authoring, with finality provided by the relay chain.

### Core functionality

The main purpose of the Bulletin chain is to provide storage for the People
Chain over the bridge.

#### Storage

The core functionality of the bulletin chain is in the transaction-storage
pallet, which indexes transactions and manages storage proofs for arbitrary
data.

Data is added via the `transactionStorage.store` extrinsic, provided the
storage of the data is authorized. Authorization is granted either for a
specific account via `authorize_account` or for data with a specific preimage
via `authorize_preimage`. Once data is stored, it can be retrieved from IPFS
with the Blake2B hash of the data.

#### Bridge to PeopleChain

For Polkadot, the bulletin chain is bridged to directly from the
proof-of-personhood chain (instead of through BridgeHub, for ease of upgrade),
allowing the PoP chain to authorize preimages for storage and allowing accounts
to store data.

#### PeopleChain integration

The PeopleChain root will call `transactionStorage.authorize_preimage` (over
the bridge) to prime Bulletin to expect data with that hash, after which a
user account will submit the data via `transactionStorage.store` (over the
bridge).

### Pallets

#### pallets/relayer-set

Controls the authorized relayers between Bulletin and PoP-polkadot.

#### pallets/validator-set

Controls the validator set. Currently set in genesis and validators can be
added and removed by root.

#### pallets/transaction-storage

Stores arbitrary data on IPFS via the `store` extrinsic, provided that either
the signer or the preimage of the data are pre-authorized. Stored data can be
retrieved from IPFS or directly from the node via the transaction index or
hash.

## Building the runtime

```bash
# Build the runtime wasm (production profile, optimized)
cargo build --profile production -p bulletin-westend-runtime --features on-chain-release-build

# Or a plain release build
cargo build --release -p bulletin-westend-runtime
```

## Running a collator / full node

The bulletin chain runs on top of the Polkadot SDK's `polkadot-omni-node`
binary. Point it at the compiled runtime wasm (via a chain spec) to launch a
collator or full node — see the Polkadot SDK documentation for
`polkadot-omni-node` for the relevant flags (relay chain, collator key, IPFS
options, etc.).

### Local chain (zombienet)

```bash
zombienet -p native spawn ./zombienet/bulletin-westend-local.toml
```

## Storage requirements

With the current configuration, the maximum storage requirement is estimated
as follows:

* Storing data for up to 2 weeks:

  $$
  2 \times 7 \times 24 \times 60 \times 60 = 1,209,600 \, \text{seconds}
  $$

  divided by a 6-second block time = **201,600 blocks**

* Each block can contain up to 8–10 MiB (based on `MaxTransactionSize = 8 MiB`
  and `BlockLength = 10 MiB`)
* Total = **1,612,800–2,016,000 MiB ≈ 1,575–1,968 GiB of storage (maximum)**

This is the maximum limit, assuming full utilization of every block for two
weeks, which is unlikely to be reached in practice.

## Fresh benchmarks

Run on the dedicated machine from the root directory:

```bash
python3 scripts/cmd/cmd.py bench --runtime bulletin-westend
```

To run all benchmarks:

```bash
python3 scripts/cmd/cmd.py bench
```

# SDK & Documentation

## 📚 Bulletin SDK

**Multi-language client SDKs** for Polkadot Bulletin Chain with complete transaction submission, automatic chunking, and DAG-PB manifest generation.

- **[Rust SDK](./sdk/rust/)** - `no_std` compatible, works in native apps and ink! smart contracts
- **[TypeScript SDK](./sdk/typescript/)** - Browser and Node.js compatible

Both SDKs provide:
- ✅ All 8 pallet operations (store, authorize, renew, refresh, remove expired)
- ✅ DAG-PB manifests (IPFS-compatible)
- ✅ Authorization management (account and preimage)
- ✅ Progress tracking via callbacks

The **TypeScript SDK** includes automatic chunking with built-in transaction submission.
The **Rust SDK** provides transaction submission via `TransactionClient` and offline chunking via `BulletinClient` (prepare-only; users submit chunks via subxt).

**Quick Start**: See [sdk/README.md](./sdk/README.md)

## 📖 Documentation

**Complete SDK Book**: [`docs/book`](./docs/book/)

The Bulletin SDK Book contains comprehensive guides including:
- Concepts (authorization, chunking, DAG-PB manifests)
- Rust SDK guide (installation, API reference, no_std usage, examples)
- TypeScript SDK guide (installation, API reference, PAPI integration, examples)
- Best practices and troubleshooting

To view the documentation locally:
```bash
cd docs/book
mdbook serve --open
```

# Examples (JavaScript-based)

The `examples/` directory contains Node.js (PJS and/or PAPI) scripts demonstrating how to interact with the Bulletin chain. For detailed setup and usage instructions, see [examples/README.md](./examples/README.md).

# Troubleshooting

## Build Bulletin Mac OS

### Algorithm file not found error

If you encounter an error similar to:

```
warning: cxx@1.0.186: In file included from /Users/ndk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cxx-1.0.186/src/cxx.cc:1:
warning: cxx@1.0.186: /Users/ndk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cxx-1.0.186/src/../include/cxx.h:2:10: fatal error: 'algorithm' file not found
warning: cxx@1.0.186:     2 | #include <algorithm>
warning: cxx@1.0.186:       |          ^~~~~~~~~~~
warning: cxx@1.0.186: 1 error generated.
error: failed to run custom build command for `cxx v1.0.186`
```

This typically means your C++ standard library headers can’t be found by the compiler. This is a toolchain setup issue.

To fix:
- Run `xcode-select --install`. 
- If it says “already installed”, reinstall them (sometimes they break after OS updates):

```bash
sudo rm -rf /Library/Developer/CommandLineTools
xcode-select --install
```

- Check the Active Developer Path: `xcode-select -p`. It should output one of: `/Applications/Xcode.app/Contents/Developer`, `/Library/Developer/CommandLineTools`
- If it’s empty or incorrect, set it manually: `sudo xcode-select --switch /Library/Developer/CommandLineTools`
- If none of the above helped, see the official Mac OS recommendations for [polkadot-sdk](https://docs.polkadot.com/develop/parachains/install-polkadot-sdk/#macos)

### dyld: Library not loaded: @rpath/libclang.dylib

This means that your build script tried to use `libclang` (from LLVM) but couldn’t find it anywhere on your system or in the `DYLD_LIBRARY_PATH`.

To fix:`brew install llvm` and 
```
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
export LD_LIBRARY_PATH="$LIBCLANG_PATH:$LD_LIBRARY_PATH"
export DYLD_LIBRARY_PATH="$LIBCLANG_PATH:$DYLD_LIBRARY_PATH"
export PATH="$(brew --prefix llvm)/bin:$PATH"
```

Now verify `libclang.dylib` exists:
- `ls "$(brew --prefix llvm)/lib/libclang.dylib"`

If that file exists all good, you can rebuild the project now: 
```
cargo clean
cargo build --release
```
