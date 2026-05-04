# Polkadot Bulletin chain

The Bulletin chain is a parachain providing distributed data storage and retrieval infrastructure for the Polkadot ecosystem. It is run using Polkadot SDK’s `polkadot-omni-node`.

> Note: the previous solochain version has been removed and is no longer maintained. Only the parachain runtime is supported.

## Runtime functionality

The Bulletin chain runtime functions to store transactions for a given period of time (currently set at 2 weeks) and provide proof of storage.

### Core functionality

The main purpose of the Bulletin chain is to provide storage for the People Chain.

#### Storage
The core functionality of the bulletin chain is in the transaction-storage pallet, which indexes transactions and manages storage proofs for arbitrary data. 

Data is added via the `transactionStorage.store` extrinsic, provided the storage of the data is authorized by root call. Authorization is granted either for a specific account via authorize_account or for data with a specific preimage via authorize_preimage. Once data is stored, it can be retrieved from IPFS with the Blake2B hash of the data.

#### PeopleChain integration
The PeopleChain root will call `transactionStorage.authorize_preimage` (over XCM) to prime Bulletin to expect data with that hash, after which a user account will submit the data via `transactionStorage.store` (over XCM).

### Pallets

####  polkadot-bulletin-chain/pallets/transaction-storage
Stores arbitrary data on IPFS via the `store` extrinsic, provided that either the signer or the preimage of the data are pre-authorized. Stored data can be retrieved from IPFS or directly from the node via the transaction index or hash.

## Fresh benchmarks

Run on the dedicated machine from the root directory:
```
python3 scripts/cmd/cmd.py bench --runtime bulletin-westend
```

To run all benchmarks:
```
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