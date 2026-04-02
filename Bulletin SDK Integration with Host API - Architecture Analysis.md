# Bulletin SDK Integration with Host API: Architecture Analysis

**Author:** Naren
**Date:** 12 Mar 2026
**Status:** Draft, for discussion

---

## Context

Products in the Polkadot Triangle run inside a sandboxed iframe or webview. The Host API mediates all interaction with the outside world: network, storage, accounts. The Bulletin SDK provides data storage and retrieval on Bulletin Chain. This document analyzes how the Bulletin SDK should integrate with the Host API sandbox model.

### The Layers

| Layer | Package | Role |
|-------|---------|------|
| Host API | `@novasamatech/host-api` | Wire protocol (SCALE-encoded messages over `postMessage`) |
| Product SDK | `@novasamatech/product-sdk` | Product-facing wrappers (accounts, storage, PAPI proxy, etc.) |
| Host SDK | `paritytech/host-sdk` (Rust) | Native host implementation (smoldot, wallet, DOTNS, IPFS) |
| Bulletin SDK | `@parity/bulletin-sdk` (this repo) | CID calculation, chunking, DAG building, transaction construction |

### Sandbox Constraints

Per the Host API Sandbox Scope Definition, products must not directly access:

- **Network** (fetch, WebSocket, etc.) — routed through Host API
- **Wallet** (`window.injectedWeb3`, etc.) — host is the wallet
- **Storage** (localStorage, IndexedDB) — routed through Host API

Any integration must respect these boundaries.

---

## The Two Approaches

### Approach A: PAPI Proxy — Heavy Product Side, Thin Host API

The Product SDK proxies PAPI requests to the host's light client. The Bulletin SDK runs inside the sandbox doing all computation (chunking, CID calculation, DAG building), and only chain interactions cross the sandbox boundary via the existing PAPI proxy.

**How it works:**

```
Product App (sandbox)
    │
    ▼
Bulletin SDK (runs in sandbox)
  - CID calculation (pure JS, multiformats)
  - Fixed-size chunking
  - DAG-PB manifest building
  - Transaction construction via PAPI TypedApi
    │
    │  PAPI calls proxied via createPapiProvider()
    │  Signing proxied via accountsProvider.getProductAccountSigner()
    ▼
Host API Transport (postMessage)
    │
    ▼
Host SDK (native)
  - smoldot light client (Bulletin Chain)
  - Wallet signing (host-wallet)
  - Transaction broadcast
    │
    ▼
Bulletin Chain
```

**Integration code (Product SDK side):**

```typescript
import { createPapiProvider, createAccountsProvider } from '@novasamatech/product-sdk';
import { createClient } from 'polkadot-api';
import { AsyncBulletinClient } from '@parity/bulletin-sdk';

// PAPI provider routes chain requests through host's smoldot
const provider = createPapiProvider({
  chainId: WellKnownChain.bulletinPolkadot,
  fallback: getWsProvider('wss://bulletin-rpc.polkadot.io'),
});
const client = createClient(provider);
const api = client.getTypedApi(bulletinDescriptor);

// Signer routes through host's wallet
const accounts = createAccountsProvider();
const signer = accounts.getProductAccountSigner(account);

// Our SDK works unchanged — chain calls go through sandbox transparently
const bulletin = new AsyncBulletinClient(api, signer, client.submit);
const result = await bulletin.store(data).send();
```

---

### Approach B: Dedicated Host API Methods — Thin Product Side, Heavy Host

The Host API exposes high-level Bulletin-specific methods (`bulletinStore`, `bulletinRetrieve`, etc.). The Product SDK is a thin wrapper. All logic (chunking, CID calculation, DAG building, submission) runs on the host side in Rust.

**How it works:**

```
Product App (sandbox)
    │
    ▼
Product SDK - Bulletin module (thin wrapper)
  - Validates input
  - Serializes request
    │
    │  hostApi.bulletinStore(data, options)
    │  hostApi.bulletinRetrieve(cid)
    ▼
Host API Transport (postMessage)
    │
    ▼
Host SDK (native)
  - Bulletin Rust SDK (chunking, CID, DAG)
  - smoldot light client
  - Wallet signing
  - Transaction broadcast
    │
    ▼
Bulletin Chain
```

**Integration code (Product SDK side):**

```typescript
import { createBulletinStore } from '@novasamatech/product-sdk';

const bulletin = createBulletinStore();

// All logic happens on the host side
const result = await bulletin.store(data, { codec: 'raw', hash: 'blake2b-256' });
const content = await bulletin.retrieve(result.cid);
```

**Host API protocol additions:**

```
bulletin_store(v1, { data, codec?, hashAlgorithm?, waitFor? }) → { cid, blockNumber, extrinsicIndex }
bulletin_store_chunked(v1, { data, chunkSize?, createManifest? }) → { manifestCid, chunkCids, totalSize }
bulletin_retrieve(v1, { cid }) → { data }
bulletin_renew(v1, { block, index }) → { newBlock, newIndex }
bulletin_authorize_account(v1, { who, transactions, bytes }) → { blockHash }
```

---

## Comparison

### Approach A: PAPI Proxy (Heavy Product Side)

#### Pros

| # | Pro | Detail |
|---|-----|--------|
| 1 | **Zero Host API changes** | Uses the existing `createPapiProvider()` infrastructure. No new Host API methods needed, no protocol versioning overhead. |
| 2 | **Existing SDK works unchanged** | Our `AsyncBulletinClient` runs as-is in the sandbox. No need to rewrite or duplicate logic in Rust. |
| 3 | **Faster iteration** | Bulletin SDK updates ship independently — just update the npm package. No coordinated Host SDK + Host API + Product SDK releases. |
| 4 | **Consistent with existing patterns** | The PAPI proxy pattern is already established. Products already use `createPapiProvider()` for chain interactions. |
| 5 | **Full flexibility for products** | Products can use the builder pattern, progress callbacks, custom chunking configs — the full SDK API surface is available. |
| 6 | **Single source of truth** | One TypeScript SDK implementation. No risk of Rust and TypeScript implementations diverging in behavior. |
| 7 | **Easier debugging** | Product developers can step through the SDK code in browser DevTools since it runs in their sandbox context. |

#### Cons

| # | Con | Detail |
|---|-----|--------|
| 1 | **Large data crosses sandbox boundary** | For chunked uploads, each chunk's PAPI `store` call sends the chunk data over `postMessage`. For a 64 MiB file, that's 64 MiB of data serialized through the message channel. |
| 2 | **Computation in sandbox** | CID calculation (hashing), DAG-PB encoding all run in the product's JS context. Could be slower than native Rust, and consumes the product's CPU/memory budget. |
| 3 | **Dependency weight** | Products must bundle `multiformats`, `@noble/hashes`, protobuf encoding, and the Bulletin SDK. Adds to sandbox bundle size. |
| 4 | **No offline/background processing** | If the product's iframe is closed, in-progress chunked uploads are lost. The host can't continue uploads independently. |
| 5 | **Retrieval requires Bitswap on host side anyway** | For `retrieve()`, the host needs to fetch blocks via Bitswap (smoldot). This can't be proxied through PAPI alone — it requires dedicated Host API support regardless. |
| 6 | **Multiple round-trips for chunked uploads** | Each chunk is a separate PAPI transaction crossing the sandbox boundary. N chunks = N round-trips through `postMessage` + smoldot. |

---

### Approach B: Dedicated Host API Methods (Heavy Host Side)

#### Pros

| # | Pro | Detail |
|---|-----|--------|
| 1 | **Minimal data over sandbox boundary** | Product sends raw data once. All chunking, CID calculation, and multi-transaction submission happen on the host side without crossing the boundary repeatedly. |
| 2 | **Native performance** | Rust SDK handles hashing (blake2b, sha256), chunking, and DAG encoding natively. Significantly faster for large files. |
| 3 | **Tiny product bundle** | Product only needs a thin wrapper — no `multiformats`, no hashing libraries, no protobuf. |
| 4 | **Host can manage long-running operations** | Chunked uploads can continue even if the product's iframe is briefly unloaded. The host owns the operation lifecycle. |
| 5 | **Retrieval is naturally host-side** | Block fetching via Bitswap already requires host-side smoldot. Having store also on the host side keeps both operations in the same layer. |
| 6 | **Simpler product code** | Products call `bulletin.store(data)` and get back a CID. No chunking config, no progress wiring, no PAPI setup. |
| 7 | **Consistent across platforms** | The same Rust SDK handles Bulletin logic on desktop (dot.li), mobile (Epoca), and web. Product code is identical everywhere. |

#### Cons

| # | Con | Detail |
|---|-----|--------|
| 1 | **New Host API methods required** | Must design, version, and maintain Bulletin-specific protocol messages. Adds to the Host API surface area. |
| 2 | **Coordinated releases** | Bulletin SDK changes require updating: Rust SDK → Host SDK → Host API protocol → Product SDK wrapper. Four packages in sync. |
| 3 | **Duplicated implementation** | The Bulletin Rust SDK and TypeScript SDK must produce identical CIDs, DAG structures, and chunking behavior. Divergence = bugs. (Though the Rust SDK already exists.) |
| 4 | **Less flexibility for products** | Products get a fixed API. Custom chunking strategies, custom codecs, or advanced progress handling require Host API changes. |
| 5 | **Harder to debug** | Product developers can't step through the storage logic — it's a black box on the host side. Errors come back as serialized messages. |
| 6 | **Large data still crosses the boundary (once)** | A 64 MiB file still needs to be sent from sandbox to host via `postMessage`. Though this is one transfer vs. N chunk transfers. |
| 7 | **Host API coupling** | Bulletin becomes a first-class Host API concern. If other chains need similar storage, each needs its own Host API methods. |

---

## Hybrid Approach (Recommended)

A pragmatic path combines both approaches based on the operation:

| Operation | Where | Rationale |
|-----------|-------|-----------|
| **Store (small, ≤ 2 MiB)** | Product side (Approach A) | Single transaction, PAPI proxy works well, data crosses boundary once |
| **Store (large, chunked)** | Host side (Approach B) | Avoid N round-trips, leverage native Rust performance for hashing |
| **Retrieve** | Host side (Approach B) | Bitswap block fetching requires host's smoldot — no alternative |
| **Authorize** | Product side (Approach A) | Simple PAPI extrinsic, no data transfer |
| **Renew** | Product side (Approach A) | Simple PAPI extrinsic, no data transfer |
| **CID calculation** | Either | Pure computation, works in both contexts |

**Phase 1 (Ship now):** Use Approach A entirely via PAPI proxy. This works today with zero Host API changes and covers all Bulletin functionality. Retrieval is deferred until Bitswap is available in smoldot.

**Phase 2 (When Bitswap lands):** Add `bulletin_retrieve` as a dedicated Host API method since it inherently requires host-side Bitswap. Evaluate whether `bulletin_store_chunked` is worth adding based on real product usage patterns and performance data from Phase 1.

---

## Open Questions

1. **What is the maximum practical file size for products?** If products mostly store small data (proofs, attestations < 2 MiB), Approach A may be sufficient permanently.
2. **Is retrieval in scope for Phase 1?** If not, Approach A covers all current needs with no Host API changes.
3. **Should the Host API expose generic Bitswap methods or Bulletin-specific methods?** Generic `bitswap_stream(cids)` is more reusable; Bulletin-specific `bulletin_retrieve(cid)` is more ergonomic.
4. **How do progress events cross the sandbox boundary?** For chunked uploads via Approach B, the host needs to stream progress back to the product via Host API subscriptions.

---

## References

- [Host API Sandbox Scope Definition](https://docs.google.com/document/d/1yqV4k7_bNgEUx-UBOEA04USSOE2jD3nUq2Hs4zsiVPw/) (Torsten Stüber, 10 Mar 2026)
- [Product SDK](https://github.com/paritytech/triangle-js-sdks/tree/main/packages/product-sdk)
- [Host SDK (Rust)](https://github.com/paritytech/host-sdk)
- [Bulletin SDK (this repo)](https://github.com/paritytech/polkadot-bulletin-chain/tree/main/sdk)
- [Bitswap Design Doc](https://docs.google.com/document/d/1_Gqh_pFiIiPg3FNumkPeE-RSuVV9i4fdD9GBQF422a8/)
- [smoldot bitswap_stream PoC](https://github.com/dmitry-markin/smoldot/pull/1)
