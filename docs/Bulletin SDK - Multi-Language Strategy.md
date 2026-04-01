# Bulletin SDK: Multi-Language Strategy and Integration Plan

## The Problem: Duplicated Protocol Logic

Today, every tool that interacts with Bulletin Chain's TransactionStorage pallet reimplements the same core logic independently:

| Tool | Language | Lines of storage code | CID logic | Chunking | DAG-PB | Error handling |
|---|---|---|---|---|---|---|
| **bulletin-deploy** | JavaScript | ~300 | Own `createCID()` | Own chunking | Own DAG-PB builder | Raw `Promise` + `subscribe` |
| **dotNS CLI** | TypeScript | ~350 | Own `createRawCid()` / `createDagPbCid()` | Own `splitBytesIntoChunks()` | Own DAG-PB builder | Raw error strings |
| **Console UI** | TypeScript | ~130 (before SDK) | Was own `lib/cid.ts` | Manual | Manual | Ad hoc |

Each reimplements:
- CID calculation (multihash creation, codec selection, hash algorithm mapping)
- Chunking (splitting data, size validation, reassembly)
- DAG-PB manifest creation (UnixFS metadata, link construction, encoding)
- Transaction submission and status tracking (signed → broadcasted → in block → finalized)
- Hashing algorithm enum conversion for the pallet's SCALE encoding
- Error handling for dispatch errors, timeouts, and reorgs

This duplication is not benign. When the pallet interface changes (e.g., `store` → `store_with_cid_config`), each tool must be updated independently. When a subtle encoding bug is found (e.g., hash algorithm enum mapping), it must be fixed in every codebase. Behaviour drifts silently — a CID that validates in one tool may not match what another tool produces.

## The Solution: One SDK, Multiple Languages

The Bulletin SDK consolidates all Bulletin Chain interaction logic into a single, tested, maintained library — available in the two languages that cover every current and foreseeable consumer:

### Why TypeScript

TypeScript is where the consumers are:

- **bulletin-deploy**: CLI tool for deploying static sites to Bulletin + DotNS. Currently ~300 lines of hand-rolled storage code that the SDK replaces entirely.
- **dotNS CLI/SDK**: The `packages/cli/src/bulletin/` directory contains ~350 lines of duplicated CID, chunking, and storage code that could be a direct `import` from `@bulletin/sdk`.
- **Console UI**: The reference web application for Bulletin Chain. Already migrated to the SDK, removing ~130 lines of manual CID calculation.
- **Product SDK adapter**: Products (pApps) running inside Nova Spektr need Bulletin storage routed through the Host API. The adapter is a thin wrapper around the same core SDK.
- **Future web tooling**: Any browser-based application, developer tool, or dashboard that reads from or writes to Bulletin Chain.

TypeScript covers the entire web and Node.js ecosystem with a single codebase.

### Why Rust

Rust is where the infrastructure is:

- **Bulletin Chain node itself**: The node binary, IPFS integration, and runtime are all Rust. Testing infrastructure that needs to submit storage transactions uses Rust.
- **Validator and operator tooling**: Scripts and services that manage storage authorization, monitor retention periods, or perform housekeeping operations are naturally written in Rust.
- **Integration testing**: End-to-end tests that exercise the pallet through actual extrinsics — not mock runtimes — benefit from a Rust SDK that shares the same type definitions.
- **Embedded / WASM environments**: The Rust SDK supports `no_std`, making it usable in constrained environments where a full Node.js runtime is not available.
- **Cross-language type consistency**: The Rust SDK's error codes, event types, and API surface serve as the canonical reference. The TypeScript SDK mirrors these, ensuring that `ErrorCode.TRANSACTION_FAILED` means the same thing in both languages.

### Why Not More Languages?

Every additional language multiplies maintenance cost. Rust + TypeScript covers:
- All current consumers (see table above)
- Server-side (Rust, Node.js)
- Browser (TypeScript/WASM)
- CLI tools (both)
- Embedded (Rust `no_std`)
- Host API products (TypeScript via adapter)

There is no current consumer that requires Go, Python, or another language. If one emerges, the Rust SDK can be compiled to WASM and wrapped with language-specific bindings, avoiding a full reimplementation.

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                    polkadot-bulletin-chain repo                  │
│                                                                  │
│  ┌─────────────────────┐    ┌──────────────────────────────┐     │
│  │   sdk/rust/          │    │   sdk/typescript/             │     │
│  │   bulletin-sdk-rust  │    │   @bulletin/sdk               │     │
│  │                      │    │                               │     │
│  │  • TransactionClient │    │  • AsyncBulletinClient        │     │
│  │  • CID calculation   │    │  • BulletinPreparer           │     │
│  │  • Error types       │    │  • CID / Chunker / DAG-PB    │     │
│  │  • Event types       │    │  • Error types + events       │     │
│  │  • no_std support    │    │  • Builder pattern API        │     │
│  └──────────┬──────────┘    └──────────────┬────────────────┘     │
│             │                              │                      │
└─────────────┼──────────────────────────────┼──────────────────────┘
              │                              │
              │                    ┌─────────┴──────────────────┐
              │                    │                            │
              ▼                    ▼                            ▼
     ┌────────────────┐  ┌────────────────┐        ┌────────────────────┐
     │  Rust consumers │  │  Direct PAPI   │        │  Host API (Product │
     │                 │  │  consumers     │        │  SDK) consumers    │
     │ • Node binary   │  │                │        │                    │
     │ • Integration   │  │ • bulletin-    │        │ @bulletin/         │
     │   tests         │  │   deploy       │        │ sdk-product        │
     │ • Validator     │  │ • dotNS CLI    │        │ (in triangle-      │
     │   tooling       │  │ • Console UI   │        │  js-sdks)          │
     │ • WASM targets  │  │ • Custom dapps │        │                    │
     └────────────────┘  └────────────────┘        └────────────────────┘
                                 │                          │
                          Direct WebSocket            Host API transport
                          to Bulletin node            (no direct connections)
```

## How Each Consumer Benefits

### bulletin-deploy

**Current state**: ~300 lines of hand-rolled storage code in `src/deploy.js` — CID creation, chunking, DAG-PB building, transaction watching with manual timeout/retry, pool account management.

**With SDK**: Replace the core storage logic with SDK calls. The pool account management and auto-authorization are deploy-specific concerns that stay, but the storage primitives become:

```javascript
import { AsyncBulletinClient } from '@bulletin/sdk'

// Replace ~200 lines of createCID + watchTransaction + storeBlock
const { cid } = await client
  .store(carChunkBytes)
  .withCodec(CidCodec.Raw)
  .withHashAlgorithm(HashAlgorithm.Sha2_256)
  .withCallback(onProgress)
  .send()
```

**What this eliminates**: `createCID()`, `toHashingEnum()`, `watchTransaction()`, manual `signSubmitAndWatch` subscription handling, dispatch error formatting, timeout logic.

### dotNS CLI / SDK

**Current state**: `packages/cli/src/bulletin/store.ts` contains ~350 lines that duplicate the SDK's functionality — `storeContentOnBulletin()`, `storeSingleFileToBulletin()`, `storeChunkedFileToBulletin()`, `storeBatchedBlocksToBulletin()`, plus CID utilities in `cid.ts`.

**With SDK**: The entire `src/bulletin/` directory can be replaced with SDK imports:

```typescript
import { AsyncBulletinClient, CidCodec, HashAlgorithm } from '@bulletin/sdk'

// storeSingleFileToBulletin → client.store(data).send()
// storeChunkedFileToBulletin → client.store(largeData).withChunkSize(1024*1024).send()
// createRawCid / createDagPbCid → calculateCid() / getContentHash()
```

**What this eliminates**: `splitBytesIntoChunks()`, `convertHashCodeToEnum()`, manual DAG-PB construction, manual transaction subscription handling, `storeContentOnBulletin()` with all its options.

### Console UI

**Current state**: Already migrated. The Console UI uses the SDK for all write operations (Upload, Renew, Authorizations) and CID calculation.

**What changed**: Removed ~130 lines of `lib/cid.ts` and manual PAPI transaction handling. All storage operations now go through the SDK's builder pattern with progress callbacks.

### Product SDK (via Host API adapter)

**Current state**: Not yet implemented. Products running inside Nova Spektr currently cannot interact with Bulletin Chain.

**With SDK adapter**: A thin `@bulletin/sdk-product` package in the `triangle-js-sdks` monorepo wires `@bulletin/sdk` to the Host API:

```typescript
import { createBulletinProductClient } from '@bulletin/sdk-product'

// All calls route through Host API — no direct WebSocket connections
const client = await createBulletinProductClient({ signer })
const { cid } = await client.store(data).send()
```

The adapter is possible because `AsyncBulletinClient` accepts abstract PAPI interfaces (`BulletinTypedApi`, `PolkadotSigner`, `SubmitFn`), not concrete providers. The Product SDK's `createPapiProvider()` returns a standard PAPI `JsonRpcProvider` that routes JSON-RPC through the Host API transport. From the SDK's perspective, it's the same interface — it doesn't know or care whether the underlying transport is a WebSocket or the Host API.

**Key constraint**: Products cannot make direct connections. All chain interactions (transaction submission, storage queries, event subscriptions) must go through the Host API. The SDK's transport-agnostic design makes this possible without any code changes to the core package.

### Rust Consumers

**Node integration tests**: The Rust SDK provides `TransactionClient` for submitting real extrinsics in end-to-end tests against a running node, with proper error types that match the pallet's error enum.

**Validator tooling**: Authorization management (granting, refreshing, revoking storage quotas) uses the Rust SDK's `authorize_account()`, `refresh_account_authorization()`, and `remove_expired_account_authorization()` methods.

## Maintaining Consistency from One Place

Both SDKs live in the same repository (`polkadot-bulletin-chain/sdk/`) alongside the pallet they interact with. This co-location is intentional:

1. **Pallet changes propagate immediately.** When the `TransactionStorage` pallet's interface changes (new extrinsic, renamed field, updated error enum), the SDKs are updated in the same PR. There is no "update the SDK later" step.

2. **Error codes are consistent.** The Rust `Error::code()` method returns the same `SCREAMING_SNAKE_CASE` string as the TypeScript `ErrorCode` enum. Adding a new error means updating both SDKs in one commit, with CI enforcing that tests pass for both.

3. **Event types mirror each other.** `TransactionStatusEvent` variants (`Validated`, `Broadcasted`, `InBestBlock`, `Finalized`, `Invalid`, `Dropped`) have the same semantics in both languages. The Rust SDK uses an enum; the TypeScript SDK uses a discriminated union with string enum discriminants.

4. **Single CI pipeline.** Both SDKs are tested in the same CI workflow. A pallet change that breaks either SDK fails the build before merge.

5. **One review process.** Contributors reviewing a pallet change can see the SDK impact in the same diff. There is no cross-repo coordination needed.

The downstream consumers (`bulletin-deploy`, `dotNS-SDK`, Product SDK adapter) are separate repositories because they have their own release cycles, dependencies, and concerns. But the core protocol logic they all depend on — CID calculation, chunking, DAG-PB encoding, transaction submission — is maintained exactly once.

## SDK Feature Matrix

| Feature | Rust SDK | TypeScript SDK |
|---|:---:|:---:|
| Store data (single transaction) | Yes | Yes |
| Store with CID config (codec + hash) | Yes | Yes |
| Automatic chunking | Via caller | Yes (built-in) |
| DAG-PB manifest generation | Via caller | Yes (built-in) |
| CID calculation | Via `cid` module | Yes (`calculateCid`, `getContentHash`) |
| Authorization management | Yes | Yes |
| Renewal | Yes | Yes |
| Transaction progress tracking | Yes (callbacks) | Yes (callbacks + builder) |
| Builder pattern | No (direct methods) | Yes |
| Error codes (consistent) | `Error::code()` | `ErrorCode` enum |
| Event types (consistent) | `TransactionStatusEvent` | `TransactionStatusEvent` |
| Retryable error classification | `is_retryable()` | `.retryable` getter |
| Recovery hints | `recovery_hint()` | `.recoveryHint` getter |
| `no_std` support | Yes | N/A |
| WASM target | Yes | N/A (runs in browser natively) |
| Host API compatibility | N/A | Yes (via adapter) |
| Offline preparation | N/A | Yes (`BulletinPreparer`) |

## Summary

The Bulletin SDK exists in two languages because the Bulletin Chain ecosystem spans two runtime environments: Rust (infrastructure, node, testing) and TypeScript (tools, web apps, products). Maintaining both SDKs in one repository alongside the pallet they serve ensures consistency, eliminates protocol drift, and gives every consumer — from a CLI deploy tool to a product running inside Nova Spektr — the same correct, tested implementation of Bulletin Chain's storage protocol.
