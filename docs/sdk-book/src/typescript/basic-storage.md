# Basic Storage

```typescript
import { BulletinClient } from '@bulletin/sdk';

// 1. Initialize Client
const client = new BulletinClient({
    endpoint: 'ws://localhost:9944'
});

// 2. Prepare Data
const data = new TextEncoder().encode('Hello, Bulletin!');

// 3. Prepare Operation
const { data: preparedData, cid } = await client.prepareStore(data);

console.log('CID:', cid.toString());

// 4. Submit (see PAPI Integration)
```
