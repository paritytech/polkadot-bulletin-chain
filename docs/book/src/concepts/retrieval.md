# Data Retrieval

The Bulletin Chain uses a **write-to-chain, read-from-network** model. Data is stored on-chain via transactions, and retrieval happens through the Bulletin validator network.

> **Important**: The SDKs have no retrieval API. Bulletin nodes and smoldot serve stored blocks through the `bitswap_v1_get` JSON-RPC method, and validator nodes serve them over Bitswap.

## Retrieval Status

| Method | Status | Description |
|--------|--------|-------------|
| **Node / smoldot `bitswap_v1_get` RPC** | Available | Raw blocks by CID, over a node RPC connection or a smoldot light client |
| **Direct P2P (Helia/libp2p)** | Available | Connect directly to validator nodes |
| **IPFS Gateways** | Deprecated | Centralized, not recommended |

## Node and Light Client RPC (`bitswap_v1_get`)

Bulletin nodes expose stored data through the `bitswap_v1_get` JSON-RPC method. smoldot serves the same method and fetches the block from its peers over Bitswap, so retrieval works without a centralized gateway.

```
┌─────────────┐     store()      ┌──────────────────┐
│   Your App  │ ───────────────► │  Bulletin Chain  │
└─────────────┘                  └──────────────────┘
       │                                  │
       │   bitswap_v1_get RPC             │ Validators serve data
       │   (node or smoldot)              │ via Bitswap protocol
       │                                  ▼
       └─────────────────────────►┌──────────────────┐
                                  │ Validator Nodes  │
                                  └──────────────────┘
```

The method returns the raw block for a CID as 0x-prefixed hex, with no DAG assembly: for a manifest root you get the DAG-PB node bytes, not the file. With the TypeScript SDK, call it through the client's own connection (WebSocket or smoldot):

```typescript
import { Binary } from "polkadot-api";

const hex = await client.request<string>("bitswap_v1_get", [cid.toString()]);
const block = Binary.fromHex(hex); // Uint8Array
```

## Direct P2P via Helia

You can also connect directly to Bulletin validator nodes using libp2p/Helia. This is **decentralized** but requires additional dependencies. Bulletin CIDs use BLAKE2b-256 by default, which Helia does not support out of the box, so pass the hasher.

### Example (Browser)

```typescript
import { createHelia } from "helia";
import { unixfs } from "@helia/unixfs";
import { webSockets } from "@libp2p/websockets";
import { noise } from "@chainsafe/libp2p-noise";
import { yamux } from "@chainsafe/libp2p-yamux";
import { multiaddr } from "@multiformats/multiaddr";
import { blake2b256 } from "@multiformats/blake2/blake2b";
import { CID } from "multiformats/cid";
import { concat } from "uint8arrays/concat";

// Bulletin validator node multiaddrs
const BULLETIN_PEERS = [
  "/dns4/westend-bulletin-rpc-node-0.polkadot.io/tcp/443/wss/p2p/12D3KooWGb3sdXpdQPvL1wwHYHpQpMAEWxpgNNb6sndHmCByMXZw",
  // Add more validator multiaddrs
];

async function fetchFromBulletin(cidString: string): Promise<Uint8Array> {
  const helia = createHelia({
    hashers: [blake2b256],
    libp2p: {
      transports: [webSockets()],
      connectionEncrypters: [noise()],
      streamMuxers: [yamux()],
    },
  });
  await helia.start();

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
  return concat(chunks);
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

For large files, the root CID points to a DAG-PB manifest. Helia (`unixfs.cat`) and gateways reassemble the chunks. The `bitswap_v1_get` RPC returns raw blocks, so parse the manifest and fetch its chunks yourself:

```typescript
import { UnixFsDagBuilder, type CID } from "@parity/bulletin-sdk";
import { Binary } from "polkadot-api";

const fetchBlock = async (cid: CID) =>
  Binary.fromHex(await client.request<string>("bitswap_v1_get", [cid.toString()]));

// 1. Fetch the manifest and read its chunk CIDs
const { chunkCids } = await new UnixFsDagBuilder().parse(await fetchBlock(rootCid));

// 2. Fetch the chunks, in order
const chunks = await Promise.all(chunkCids.map(fetchBlock));
```

## Data Availability

### Retention Period

Data stored on Bulletin Chain is retained for a configurable **retention period** (query the `TransactionStorage.RetentionPeriod` storage value; default 201,600 blocks). After this period:

- The on-chain transaction data may be pruned
- Validator nodes may no longer serve the data
- Use [renewal](./renewal.md) to extend retention if needed

### Ensuring Long-Term Availability

For critical data that must outlive the retention period:

1. **Renew before expiration** - Use the SDK's renewal functionality
2. **Run your own node** - Run a Bulletin node with `--ipfs-server` to serve your data
3. **Replicate externally** - Store copies in other systems as backup

## Verifying Data Integrity

A CID contains the hash of its block. After retrieving a raw block, hash it with the CID's hash algorithm and compare the result with the CID's digest:

```typescript
import { getContentHash, type HashAlgorithm } from "@parity/bulletin-sdk";
import { CID } from "multiformats/cid";

const cid = CID.parse(cidString);
const block = await fetchBlock(cid); // raw block, e.g. from bitswap_v1_get

const digest = await getContentHash(block, cid.multihash.code as HashAlgorithm);
const expected = cid.multihash.digest;
const verified =
  digest.length === expected.length && digest.every((byte, i) => byte === expected[i]);

console.log(verified ? "Data integrity verified!" : "Data corruption detected!");
```

## SDK Roadmap

The SDKs do not wrap retrieval yet. Planned:

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
