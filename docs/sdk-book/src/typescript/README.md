# TypeScript SDK

The `@bulletin/sdk` package provides a client for Node.js and browser environments, integrated with Polkadot API (PAPI).

## Quick Example

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';

const client = new AsyncBulletinClient(api, signer);
const result = await client.store(data).send();
console.log('CID:', result.cid.toString());
```

> **Note**: `store().send()` is not yet fully implemented. Authorization operations and CID/chunking/manifest generation work. See the [examples](../../../../examples/) directory for patterns using PAPI directly.

## Guides

- [Installation](./installation.md)
- [Basic Storage](./basic-storage.md)
- [Chunked Uploads](./chunked-uploads.md)
- [PAPI Integration](./papi-integration.md)
