# Data Retrieval

The Bulletin Chain uses a **write-to-chain, read-from-network** model. Data is stored on-chain via transactions, and retrieval happens through the Bulletin validator network.

> **Important**: The SDK currently focuses on **storage operations only**. Data retrieval functionality will be added in a future release using the smoldot light client.

## Retrieval Status

| Method | Status | Description |
|--------|--------|-------------|
| **Smoldot `bitswap_block`** | Coming Soon | Decentralized retrieval via light client |
| **Direct P2P (Helia/libp2p)** | Available | Connect directly to validator nodes |
| **IPFS Gateways** | Deprecated | Centralized, not recommended |

## Future: Smoldot Light Client Retrieval (Recommended)

Data retrieval will be supported via the smoldot light client's `bitswap_block` RPC. This approach allows **fully decentralized** data retrieval directly from Bulletin validator nodes without relying on centralized gateways.

```
┌─────────────┐     store()      ┌──────────────────┐
│   Your App  │ ───────────────► │  Bulletin Chain  │
└─────────────┘                  └──────────────────┘
       │                                  │
       │   bitswap_block RPC              │ Validators serve data
       │   (via smoldot)                  │ via Bitswap protocol
       │                                  ▼
       └─────────────────────────►┌──────────────────┐
                                  │ Validator Nodes  │
                                  └──────────────────┘
```

### How It Will Work

```javascript
// Future SDK API (not yet available)
import { retrieve } from "@bulletin/sdk";

// Retrieves data via smoldot's bitswap_block RPC
const data = await retrieve(client, cid);
```

Under the hood, this will use smoldot's custom `bitswap_block` RPC:

```javascript
// Low-level API (requires smoldot-bitswap fork)
const result = await client._request("bitswap_block", [cidString]);
const data = hexToBytes(result.slice(2)); // Strip "0x" prefix
```

See the development progress: [PR #264](https://github.com/paritytech/polkadot-bulletin-chain/pull/264)

## Current Workaround: Direct P2P via Helia

For applications that need retrieval now, you can connect directly to Bulletin validator nodes using libp2p/Helia. This is **decentralized** but requires additional dependencies.

### Example (Browser)

```typescript
import { createHelia } from "helia";
import { unixfs } from "@helia/unixfs";
import { webSockets } from "@libp2p/websockets";
import { noise } from "@chainsafe/libp2p-noise";
import { yamux } from "@chainsafe/libp2p-yamux";

// Bulletin validator node multiaddrs
const BULLETIN_PEERS = [
  "/dns4/bulletin-westend-rpc.polkadot.io/tcp/443/wss/p2p/12D3KooW...",
  // Add more validator multiaddrs
];

async function fetchFromBulletin(cidString: string): Promise<Uint8Array> {
  const helia = await createHelia({
    libp2p: {
      transports: [webSockets()],
      connectionEncrypters: [noise()],
      streamMuxers: [yamux()],
    },
  });

  // Connect to Bulletin validators
  for (const addr of BULLETIN_PEERS) {
    await helia.libp2p.dial(multiaddr(addr));
  }

  const fs = unixfs(helia);
  const chunks: Uint8Array[] = [];

  for await (const chunk of fs.cat(CID.parse(cidString))) {
    chunks.push(chunk);
  }

  await helia.stop();
  return concatenate(chunks);
}
```

See the console-ui implementation for a complete reference: `console-ui/src/lib/helia.ts`

## Deprecated: IPFS Gateway Retrieval

> **Warning**: Public IPFS gateways are **centralized infrastructure** and go against the decentralization goals of Bulletin Chain. This method is **deprecated** and not recommended for production use.

Public gateways like `ipfs.io`, `cloudflare-ipfs.com`, etc. are:
- Centralized single points of failure
- Subject to rate limits and availability issues
- Not guaranteed to have Bulletin Chain data

If you must use gateways temporarily:

```javascript
// DEPRECATED - use only as last resort
const gateway = "https://ipfs.io";
const response = await fetch(`${gateway}/ipfs/${cid}`);
const data = await response.arrayBuffer();
```

## Retrieving Chunked Data

For large files stored via `prepare_store_chunked`, the root CID points to a DAG-PB manifest. The retrieval method (smoldot, Helia, or gateway) will automatically reassemble the chunks:

```javascript
// Root CID automatically resolves to complete file
const fullFile = await retrieve(client, rootCid);
```

For manual chunk retrieval (streaming, partial downloads):

```javascript
// 1. Fetch manifest to get chunk CIDs
const manifest = await retrieveManifest(client, rootCid);

// 2. Fetch individual chunks
for (const link of manifest.Links) {
  const chunkCid = link.Hash;
  const chunk = await retrieve(client, chunkCid);
  // Process chunk...
}
```

## Data Availability

### Retention Period

Data stored on Bulletin Chain is retained for a configurable **retention period** (check chain constants for the current value). After this period:

- The on-chain transaction data may be pruned
- Validator nodes may no longer serve the data
- Use [renewal](./renewal.md) to extend retention if needed

### Ensuring Long-Term Availability

For critical data that must outlive the retention period:

1. **Renew before expiration** - Use the SDK's renewal functionality
2. **Run your own node** - Run a Bulletin node with `--ipfs-server` to serve your data
3. **Replicate externally** - Store copies in other systems as backup

## Verifying Data Integrity

The CID includes a cryptographic hash of the content. After retrieval, verify the data matches:

```typescript
import { calculateCid } from "@bulletin/sdk";
import { CID } from "multiformats/cid";

// After retrieving data
const retrievedData = await retrieve(client, cidString);

// Verify integrity
const computedCid = await calculateCid(retrievedData);
const originalCid = CID.parse(cidString);

if (computedCid.equals(originalCid)) {
  console.log("Data integrity verified!");
} else {
  console.error("Data corruption detected!");
}
```

## SDK Roadmap

The SDK will support retrieval once the smoldot `bitswap_block` RPC is production-ready:

| Feature | Status |
|---------|--------|
| `retrieve(client, cid)` | Planned |
| `retrieveChunked(client, cid)` | Planned |
| `verifyIntegrity(cid, data)` | Planned |
| Streaming support | Planned |

## Next Steps

- [Storage Model](./storage.md) - Understanding how data is stored
- [Manifests](./manifests.md) - DAG-PB format for chunked data
- [Renewal](./renewal.md) - Extending data retention
