# Data Retrieval

The Bulletin Chain uses a **write-to-chain, read-from-IPFS** model. Data is stored on-chain via transactions, but retrieval happens through the IPFS network.

## How It Works

```
┌─────────────┐     store()      ┌──────────────────┐
│   Your App  │ ───────────────► │  Bulletin Chain  │
└─────────────┘                  └──────────────────┘
       │                                  │
       │                                  │ Bitswap/IPFS
       │                                  ▼
       │   fetch by CID           ┌──────────────────┐
       └─────────────────────────►│   IPFS Gateway   │
                                  └──────────────────┘
```

1. **Store**: Use the SDK to store data on-chain. You get back a **CID** (Content Identifier).
2. **Retrieve**: Use the CID to fetch data from any IPFS gateway or directly from a Bulletin node running with `--ipfs-server`.

## Why This Architecture?

- **Efficiency**: IPFS is optimized for content distribution; blockchains are optimized for consensus.
- **Scalability**: Data can be served by any IPFS node, not just Bulletin validators.
- **Compatibility**: Standard IPFS tools work out of the box.
- **Availability**: Data remains accessible via IPFS even during chain upgrades.

## Retrieval Methods

### 1. IPFS HTTP Gateway (Recommended)

The simplest way to retrieve data is through an IPFS HTTP gateway:

```javascript
// JavaScript/TypeScript
const cid = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
const gateway = "https://ipfs.io";

const response = await fetch(`${gateway}/ipfs/${cid}`);
const data = await response.arrayBuffer();
```

```rust
// Rust (using reqwest)
let cid = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
let gateway = "https://ipfs.io";

let url = format!("{}/ipfs/{}", gateway, cid);
let data = reqwest::get(&url).await?.bytes().await?;
```

```bash
# curl
curl "https://ipfs.io/ipfs/bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"
```

### 2. Direct from Bulletin Node

If you're running a Bulletin node with the `--ipfs-server` flag, you can fetch data directly via Bitswap:

```bash
# Start node with IPFS server enabled
./polkadot-bulletin-chain --ipfs-server --chain bulletin-paseo

# The node exposes Bitswap on its libp2p port
# Connect your IPFS client to the node's multiaddr
```

### 3. Local IPFS Node

If you run a local IPFS node, it can fetch from the Bulletin network:

```bash
# Ensure your IPFS node can discover Bulletin nodes (via DHT or direct peering)
ipfs cat bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi
```

## Public IPFS Gateways

Here are some public gateways you can use:

| Gateway | URL |
|---------|-----|
| IPFS.io | `https://ipfs.io/ipfs/{cid}` |
| Cloudflare | `https://cloudflare-ipfs.com/ipfs/{cid}` |
| Pinata | `https://gateway.pinata.cloud/ipfs/{cid}` |
| w3s.link | `https://w3s.link/ipfs/{cid}` |

> **Note**: Public gateways may have rate limits. For production use, consider running your own gateway or using a dedicated service.

## Retrieving Chunked Data

For large files stored via `prepare_store_chunked`, the root CID points to a DAG-PB manifest. IPFS gateways automatically reassemble the chunks:

```javascript
// The gateway handles reassembly automatically
const rootCid = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
const response = await fetch(`https://ipfs.io/ipfs/${rootCid}`);
const fullFile = await response.arrayBuffer(); // Reassembled automatically
```

If you need to manually retrieve chunks (e.g., for streaming or partial downloads):

```javascript
// 1. Fetch the manifest to get chunk CIDs
const manifestResponse = await fetch(`https://ipfs.io/ipfs/${rootCid}?format=dag-json`);
const manifest = await manifestResponse.json();

// 2. Fetch individual chunks
for (const link of manifest.Links) {
    const chunkCid = link.Hash["/"];
    const chunk = await fetch(`https://ipfs.io/ipfs/${chunkCid}`);
    // Process chunk...
}
```

## Complete Example: Store and Retrieve

### TypeScript

```typescript
import { BulletinClient, CidCodec, HashAlgorithm } from "@polkadot-bulletin/sdk";

// Store
const client = new BulletinClient();
const data = new TextEncoder().encode("Hello, Bulletin!");
const operation = client.prepareStore(data);
const cidBytes = operation.calculateCid();

// Submit transaction (via PAPI)
// ... submit operation.data to chain ...

// Convert CID bytes to string for retrieval
import { CID } from "multiformats/cid";
const cid = CID.decode(cidBytes);
const cidString = cid.toString();

// Retrieve via gateway
const gateway = "https://ipfs.io";
const response = await fetch(`${gateway}/ipfs/${cidString}`);
const retrieved = new Uint8Array(await response.arrayBuffer());

console.log(new TextDecoder().decode(retrieved)); // "Hello, Bulletin!"
```

### Rust

```rust
use bulletin_sdk_rust::prelude::*;

// Store
let client = BulletinClient::new();
let data = b"Hello, Bulletin!".to_vec();
let operation = client.prepare_store(data, None)?;
let cid_data = operation.calculate_cid()?;
let cid_bytes = cid_data.to_bytes().unwrap();

// Submit transaction (via subxt)
// ... submit operation.data() to chain ...

// Convert CID to string for retrieval
let cid = cid::Cid::try_from(cid_bytes.as_slice())?;
let cid_string = cid.to_string();

// Retrieve via gateway (using reqwest)
let gateway = "https://ipfs.io";
let url = format!("{}/ipfs/{}", gateway, cid_string);
let retrieved = reqwest::get(&url).await?.bytes().await?;

println!("{}", String::from_utf8_lossy(&retrieved)); // "Hello, Bulletin!"
```

## Data Availability

### Retention Period

Data stored on Bulletin Chain is retained for a configurable **retention period** (check chain constants for the current value). After this period:

- The on-chain transaction data may be pruned
- IPFS nodes that have cached the data may still serve it
- For long-term availability, consider pinning to a dedicated IPFS pinning service

### Ensuring Availability

For critical data, consider:

1. **Pinning Services**: Pin your CIDs to services like Pinata, web3.storage, or Filebase
2. **Self-Hosting**: Run your own IPFS node and pin important data
3. **Multiple Gateways**: Don't rely on a single gateway for retrieval

## Verifying Data Integrity

The CID includes a cryptographic hash of the content. When you retrieve data, you can verify it matches:

```typescript
import { CID } from "multiformats/cid";
import { sha256 } from "multiformats/hashes/sha2";

const retrieved = await fetch(`https://ipfs.io/ipfs/${cidString}`);
const data = new Uint8Array(await retrieved.arrayBuffer());

// Recompute hash and verify
const hash = await sha256.digest(data);
const computedCid = CID.create(1, 0x55, hash); // 0x55 = raw codec

if (computedCid.toString() === cidString) {
    console.log("Data integrity verified!");
}
```

## SDK Roadmap

> **Note**: Currently the SDK focuses on storage operations. Retrieval helpers are planned for future releases:
>
> - `retrieve(cid, gateway)` - Fetch data from IPFS gateway
> - `retrieveChunked(cid, gateway)` - Stream chunked data
> - `verifyIntegrity(cid, data)` - Verify data matches CID

For now, use standard HTTP clients or IPFS libraries for retrieval as shown above.

## Next Steps

- [Storage Model](./storage.md) - Understanding how data is stored
- [Manifests & IPFS](./manifests.md) - DAG-PB format details
- [Chunked Uploads (Rust)](../rust/chunked-uploads.md) - Storing large files
- [Chunked Uploads (TypeScript)](../typescript/chunked-uploads.md) - Storing large files
