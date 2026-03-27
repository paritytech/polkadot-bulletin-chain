# RFC: Data Retrieval API for Bulletin TypeScript SDK

**Status**: Draft
**Author**: @mudigal
**Date**: 2026-03-04
**Branch**: `feat/sdk-read-support`
**Related**: [PR #264 (Smoldot Bitswap)](https://github.com/paritytech/polkadot-bulletin-chain/pull/264)

---

## Summary

Add first-class data retrieval support to the Bulletin TypeScript SDK (`@bulletin/sdk`) with two pluggable provider backends:

1. **IPFS Gateway Provider** — HTTP fetch from any IPFS-compatible gateway (zero new dependencies)
2. **Helia P2P Provider** — Direct Bitswap connection to Bulletin validator nodes via Helia/libp2p (optional peer dependencies)

Both providers implement a common `RetrievalProvider` interface, enabling a unified retrieval API with automatic DAG-PB manifest resolution, parallel chunk fetching, and CID verification.

---

## Motivation

### Current State

The Bulletin SDK currently only supports **storage (write) operations**. The SDK explicitly states:

> *"This SDK currently does NOT provide data retrieval functionality."*

Applications that need retrieval must build their own implementation. The console-ui already has a working Helia P2P client (`console-ui/src/lib/helia.ts`), but it's tightly coupled to the UI and not reusable as a library.

### Why Now?

- **Bitswap is production-ready**: The Bulletin Chain node fully supports Bitswap via `litep2p` backend with the `--ipfs-server` flag. Validators serve stored data over the Bitswap protocol.
- **Working reference implementation**: The console-ui's Helia client has been proven in production across local, Westend, and Paseo networks.
- **Developer experience gap**: Storing data without a way to retrieve it in the same SDK creates a fragmented developer experience.
- **Smoldot Bitswap not yet available**: The ideal future solution (smoldot + `bitswap_block` RPC) is still pending. Adding retrieval now with a provider abstraction makes it easy to add a Smoldot provider later without breaking the API.

---

## Architecture

### Provider Pattern

```
                     User API
                        │
               RetrieveBuilder (fluent)
                        │
                 retrieveData() orchestrator
                ┌───────┴────────┐
         IpfsGatewayProvider   HeliaProvider
         (built-in fetch)      (optional Helia)
                                     │
                              ┌──────┴──────┐
                         Future providers:
                         SmoldotProvider
                         CustomProvider
```

The architecture separates **how** data is fetched (providers) from **what** happens after (manifest resolution, chunk reassembly, verification). Providers only need to implement one method: `fetchBlock(cid) → Uint8Array`.

### Core Interface

```typescript
interface RetrievalProvider {
  /** Human-readable name for logging/diagnostics */
  readonly name: string;

  /** Initialize the provider (connect to peers, validate gateway, etc.) */
  initialize(): Promise<void>;

  /** Fetch a single raw block by CID */
  fetchBlock(cid: CID, options?: { timeoutMs?: number }): Promise<Uint8Array>;

  /** Whether the provider is initialized and ready */
  isReady(): boolean;

  /** Clean up resources (close connections, etc.) */
  shutdown(): Promise<void>;
}
```

This interface is intentionally minimal. A provider only needs to fetch individual blocks — the orchestration layer handles everything else.

---

## Providers

### IPFS Gateway Provider

**Dependencies**: None (uses built-in `fetch()`)

```typescript
const provider = new IpfsGatewayProvider({
  gatewayUrl: "http://127.0.0.1:8283"   // Bulletin node's built-in gateway
});
await provider.initialize();

const result = await retrieveData(provider, parseCid("bafkrei..."));
```

**Implementation**:
- `fetchBlock(cid)` → `GET ${gatewayUrl}/ipfs/${cid}?format=raw`
- Uses `AbortController` for timeout support
- The `?format=raw` parameter ensures the gateway returns raw block bytes (important for dag-pb manifests)
- Works with Bulletin's built-in gateway (port 8283) or any standard IPFS gateway

**Trade-offs**:
- (+) Zero dependencies, simplest to use
- (+) Works everywhere (browser, Node.js, Deno)
- (-) Requires a running gateway (centralized point)
- (-) Public IPFS gateways may not have Bulletin data

### Helia P2P Provider

**Dependencies**: Optional peer dependencies (`helia`, `@multiformats/blake2`, `@noble/hashes`, `@multiformats/multiaddr`)

```typescript
const provider = new HeliaProvider({
  peerMultiaddrs: [
    "/dns4/westend-bulletin-collator-node-0.parity-testnet.parity.io/tcp/443/wss/p2p/12D3KooW..."
  ]
});
await provider.initialize();

const result = await retrieveData(provider, parseCid("bafkrei..."));
await provider.shutdown();
```

**Implementation**:
- Uses **dynamic `import()`** for all Helia dependencies — base SDK bundle is not affected
- Creates a Helia libp2p node with three hashers: blake2b-256, sha2-256, keccak-256
- **Peer whitelisting**: Only connects to specified validator nodes via `connectionGater`
- `fetchBlock(cid)` → `helia.blockstore.get(cid)` with timeout
- Handles both `Uint8Array` and async iterable responses from the blockstore
- `getConnections()` method for diagnostics

**Trade-offs**:
- (+) Fully decentralized — connects directly to validators
- (+) No centralized gateway dependency
- (+) Matches production console-ui implementation
- (-) Heavier dependencies (~2MB when installed)
- (-) Requires WebSocket-capable validator endpoints

### Future: Smoldot Provider

When the smoldot `bitswap_block` RPC is implemented (PR #264), a `SmoldotProvider` can be added without any changes to the retrieval API:

```typescript
// Future — not part of this PR
class SmoldotProvider implements RetrievalProvider {
  fetchBlock(cid) {
    return this.client._request("bitswap_block", [cid.toString()]);
  }
}
```

---

## Orchestration Layer

The `retrieveData()` function is the core of the retrieval system. It's provider-agnostic and handles:

### 1. DAG-PB Manifest Auto-Detection

When the CID has codec `0x70` (dag-pb), the orchestrator:
1. Fetches the manifest block
2. Parses it via the existing `UnixFsDagBuilder.parse()` method
3. Extracts chunk CIDs and total size
4. Fetches all chunks in parallel
5. Reassembles using the existing `reassembleChunks()` function

Users can opt out with `withManifest(false)` to get raw manifest bytes.

### 2. Parallel Chunk Fetching

Uses the existing `limitConcurrency()` utility with configurable parallelism (default: 8). Each chunk fetch is wrapped in `retry()` (default: 2 retries with exponential backoff).

**Order preservation**: Since `limitConcurrency()` doesn't preserve insertion order, chunks are collected as `{index, data}` tuples, sorted by index, then converted to `Chunk[]` for `reassembleChunks()`.

### 3. CID Verification

After retrieval, the data's CID is recalculated using `calculateCid()` and compared to the requested CID. This detects data corruption or tampering. Enabled by default, can be disabled with `withVerification(false)`.

### 4. Progress Events

```typescript
type RetrievalProgressEvent =
  | { type: "retrieval_started"; cid: string }
  | { type: "manifest_detected"; totalChunks: number; totalSize: number }
  | { type: "chunk_fetch_started"; index: number; total: number; cid: string }
  | { type: "chunk_fetch_completed"; index: number; total: number; cid: string; size: number }
  | { type: "chunk_fetch_failed"; index: number; total: number; cid: string; error: Error }
  | { type: "verification_started" }
  | { type: "verification_completed"; valid: boolean }
  | { type: "retrieval_completed"; size: number; elapsed: number }
```

---

## User-Facing API

### Standalone Usage

```typescript
import { IpfsGatewayProvider, HeliaProvider, retrieveData, parseCid } from "@bulletin/sdk";

// Gateway
const gateway = new IpfsGatewayProvider({ gatewayUrl: "http://127.0.0.1:8283" });
await gateway.initialize();
const result = await retrieveData(gateway, parseCid("bafkrei..."));
console.log(result.data);  // Uint8Array

// Helia P2P
const helia = new HeliaProvider({
  peerMultiaddrs: ["/ip4/127.0.0.1/tcp/30334/ws/p2p/12D3KooW..."]
});
await helia.initialize();
const result = await retrieveData(helia, parseCid("bafkrei..."), {
  verify: true,
  timeoutMs: 60_000
});
await helia.shutdown();
```

### Builder Pattern (via AsyncBulletinClient)

```typescript
const client = new AsyncBulletinClient(api, signer, {
  retrievalProvider: new IpfsGatewayProvider({ gatewayUrl: "http://127.0.0.1:8283" })
});

// Simple
const result = await client.retrieve("bafkrei...").fetch();

// With options
const result = await client
  .retrieve(storeResult.cid)
  .withTimeout(60_000)
  .withVerification(true)
  .withParallel(4)
  .withRetries(3)
  .withCallback((event) => {
    if (event.type === "chunk_fetch_completed") {
      console.log(`Chunk ${event.index + 1}/${event.total} fetched`);
    }
  })
  .fetch();

console.log(result.data);       // Uint8Array — original file
console.log(result.isChunked);  // true if resolved from manifest
console.log(result.verified);   // true if CID verification passed
console.log(result.elapsed);    // milliseconds
```

### Store + Retrieve Round-Trip

```typescript
// Store
const storeResult = await client.store(myData).send();

// Later, retrieve
const retrieveResult = await client.retrieve(storeResult.cid).fetch();
assert(deepEqual(myData, retrieveResult.data));  // ✓
```

### Chunked Data (Automatic)

```typescript
// Store large file — SDK auto-chunks and creates DAG-PB manifest
const storeResult = await client.store(largeFile).send();
// storeResult.cid is a dag-pb manifest CID (codec 0x70)

// Retrieve — auto-detects manifest, fetches all chunks, reassembles
const result = await client.retrieve(storeResult.cid).fetch();
// result.data === largeFile (reassembled)
// result.isChunked === true
// result.chunks.numChunks === N

// Get raw manifest bytes instead
const manifest = await client.retrieve(storeResult.cid).withManifest(false).fetch();
```

---

## Return Type

```typescript
interface RetrieveResult {
  /** The retrieved data bytes */
  data: Uint8Array;
  /** The CID that was requested */
  cid: CID;
  /** Total size in bytes */
  size: number;
  /** Whether data was resolved from a DAG-PB manifest */
  isChunked: boolean;
  /** Chunk details (only present if chunked) */
  chunks?: {
    chunkCids: CID[];
    numChunks: number;
    manifestTotalSize: number;
  };
  /** Retrieval time in milliseconds */
  elapsed: number;
  /** Whether CID verification passed */
  verified: boolean;
}
```

---

## Dependency Strategy

### IPFS Gateway Provider
**Zero new dependencies.** Uses the built-in `fetch()` API (available in Node.js 18+ and all modern browsers).

### Helia Provider
**Optional peer dependencies with dynamic imports:**

```json
{
  "peerDependencies": {
    "helia": "^6.0.0",
    "@multiformats/blake2": "^2.0.0",
    "@noble/hashes": "^1.7.0",
    "@multiformats/multiaddr": "^13.0.0"
  },
  "peerDependenciesMeta": {
    "helia": { "optional": true },
    "@multiformats/blake2": { "optional": true },
    "@noble/hashes": { "optional": true },
    "@multiformats/multiaddr": { "optional": true }
  }
}
```

All Helia imports use `await import("helia")` (dynamic). If a user tries to use `HeliaProvider` without installing these packages, they get a clear error:

```
BulletinError: Helia dependencies not installed.
Run: npm install helia @multiformats/blake2 @noble/hashes @multiformats/multiaddr
Code: MISSING_DEPENDENCY
```

Users who only need the gateway provider pay zero bundle cost for Helia.

---

## Code Reuse

The retrieval system reuses significant existing SDK infrastructure:

| Existing Code | Location | Reused For |
|---|---|---|
| `UnixFsDagBuilder.parse()` | `dag.ts:95-127` | Parse DAG-PB manifests to extract chunk CIDs |
| `reassembleChunks()` | `chunker.ts` | Reconstruct original data from fetched chunks |
| `parseCid()` | `utils.ts:84` | Parse CID strings from user input |
| `calculateCid()` | `utils.ts:51` | Verify retrieved data matches expected CID |
| `limitConcurrency()` | `utils.ts:328` | Parallel chunk fetching with concurrency limit |
| `retry()` | `utils.ts:245` | Per-chunk retry with exponential backoff |
| `measureTime()` | `utils.ts:409` | Track retrieval elapsed time |
| `BulletinError` | `types.ts:215` | Consistent error handling |
| `ProgressCallback` | `types.ts:210` | Progress event reporting |

---

## Files Changed

| File | Action | Description |
|---|---|---|
| `src/types.ts` | Modify | Add `RetrievalProvider`, `RetrieveResult`, `RetrieveOptions`, progress events |
| `src/ipfs-gateway-provider.ts` | **New** | IPFS HTTP gateway provider |
| `src/helia-provider.ts` | **New** | Helia P2P Bitswap provider |
| `src/retrieval.ts` | **New** | Core orchestration (manifest resolution, chunks, verification) |
| `src/retrieve-builder.ts` | **New** | Fluent builder API |
| `src/async-client.ts` | Modify | Add `.retrieve()` method + `retrievalProvider` config |
| `src/mock-client.ts` | Modify | Add mock retrieval for testing |
| `src/index.ts` | Modify | Export new modules, update docstring |
| `package.json` | Modify | Add optional peer dependencies |
| `test/unit/retrieval.test.ts` | **New** | Orchestration tests (mock provider) |
| `test/unit/ipfs-gateway-provider.test.ts` | **New** | Gateway provider tests (mock fetch) |
| `test/unit/retrieve-builder.test.ts` | **New** | Builder tests |

---

## Testing Strategy

### Unit Tests (no network required)

- **Mock `RetrievalProvider`** that returns pre-defined blocks by CID
- **Mocked `fetch()`** via vitest for gateway provider tests
- Test cases:
  - Single block retrieval (raw codec)
  - DAG-PB manifest detection and chunk reassembly
  - CID verification — pass and mismatch scenarios
  - Progress event emission sequence
  - Timeout behavior
  - Error handling (provider not initialized, fetch failure, manifest parse failure)
  - Builder option propagation

### Integration Tests (require running node)

- End-to-end store + retrieve via IPFS gateway
- Chunked store + retrieve round-trip
- Helia P2P retrieval from local validator node

---

## Open Questions

1. **Should the SDK ship a default list of known validator multiaddrs per network?** The console-ui hardcodes these in `Download.tsx`. Having them in the SDK would simplify the Helia provider setup, but they'd go stale.

2. **Streaming retrieval for large files?** The current design buffers the entire file in memory. For very large files, a streaming API (`retrieveStream()` returning `ReadableStream<Uint8Array>`) could be added later.

3. **Should `MockBulletinClient.store()` automatically populate the mock retrieval store?** This would make round-trip testing seamless (store → retrieve in tests without manual setup).

---

## Future Work

- **Smoldot Provider**: When `bitswap_block` RPC lands (PR #264), add `SmoldotProvider` implementing the same interface
- **Streaming retrieval**: `retrieveStream()` for large file download without full buffering
- **Rust SDK retrieval**: Mirror this API in the Rust SDK
- **Console-UI migration**: Replace console-ui's custom Helia client with the SDK's `HeliaProvider`
