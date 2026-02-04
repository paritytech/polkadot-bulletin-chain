# Basic Storage

This guide shows how to store data using the `AsyncBulletinClient` with direct PAPI integration.

## Quick Start

The `store()` method with builder pattern automatically handles both small and large files:

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';
import { createClient, Binary } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

// 1. Connect to Bulletin Chain
const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(bulletinDescriptor);

// 2. Create client with PAPI client and signer
const client = new AsyncBulletinClient(api, signer);

// 3. Store data using builder pattern (automatically chunks if > 2 MiB)
const data = Binary.fromText('Hello, Bulletin Chain!');
const result = await client.store(data).send();

console.log('✅ Stored successfully!');
console.log('   CID:', result.cid.toString());
console.log('   Size:', result.size, 'bytes');
```

## Step-by-Step Explanation

### 1. Setup Connection

First, create a PAPI client and get the typed API:

```typescript
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

// Connect to chain
const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);

// Get typed API (requires chain descriptors)
const api = papiClient.getTypedApi(bulletinDescriptor);
```

### 2. Create Client

Create the SDK client with PAPI client and signer:

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';

const client = new AsyncBulletinClient(api, signer);
```

### 3. Prepare Data

Use PAPI's `Binary` class to handle data:

```typescript
import { Binary } from 'polkadot-api';

// From text
const data = Binary.fromText('Hello, Bulletin!');

// From hex string
const data = Binary.fromHex('0x48656c6c6f');

// From Uint8Array
const data = Binary.fromBytes(new Uint8Array([72, 101, 108, 108, 111]));

// From Buffer (Node.js)
const data = Binary.fromBytes(Buffer.from('Hello'));
```

### 4. Store Data

The `store()` method with builder pattern handles everything:
- Validates data size
- Checks authorization (if configured)
- Automatically chunks large files (> 2 MiB by default)
- Calculates CID(s)
- Submits transaction(s)
- Waits for finalization

```typescript
// Basic store
const result = await client.store(data).send();

// With custom options
const result = await client
    .store(data)
    .withCodec(CidCodec.Raw)
    .withHashAlgorithm('blake2b-256')
    .withFinalization(true)
    .send();

// With progress tracking for large files
const result = await client
    .store(data)
    .withCallback((event) => {
        if (event.type === 'chunk_completed') {
            console.log(`Chunk ${event.index + 1}/${event.total} uploaded`);
        } else if (event.type === 'completed') {
            console.log('Upload complete!');
        }
    })
    .send();
```

### 5. Handle Result

```typescript
console.log('CID:', result.cid.toString());
console.log('Size:', result.size, 'bytes');
console.log('Block:', result.blockNumber);

// If chunked, check chunk details
if (result.chunks) {
    console.log('Chunks:', result.chunks.numChunks);
    console.log('Chunk CIDs:', result.chunks.chunkCids.map(c => c.toString()));
}
```

## Authorization Checking (Fail Fast)

By default, the SDK checks authorization **before** uploading to fail fast and avoid wasted transaction fees.

### How It Works

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';
import { Binary } from 'polkadot-api';

// 1. Create client with your account
const account = 'your-account-address';

const client = new AsyncBulletinClient(api, signer)
    .withAccount(account);  // Set the account for auth checking

// 2. Upload - authorization is checked automatically
const data = Binary.fromText('Hello, Bulletin!');
const result = await client.store(data).send();
//                       ⬆️ Queries blockchain first, fails fast if insufficient auth
```

### What Gets Checked

Before submitting the transaction, the SDK:
1. **Queries** the blockchain for your current authorization
2. **Validates** you have enough transactions and bytes authorized
3. **Checks expiration** - fails if authorization has expired
4. **Fails immediately** if insufficient (no transaction fees wasted!)
5. **Proceeds** only if authorization is sufficient

### Disable Authorization Checking

If you want to skip the check (e.g., you know authorization exists):

```typescript
const client = new AsyncBulletinClient(api, signer, {
    checkAuthorizationBeforeUpload: false,  // Disable checking
}).withAccount(account);
```

### Error Example

```typescript
import { BulletinError } from '@bulletin/sdk';

try {
    const result = await client.store(data).send();
    console.log('Success!');
} catch (error) {
    if (error instanceof BulletinError) {
        if (error.code === 'INSUFFICIENT_AUTHORIZATION') {
            console.error('Need more authorization!');
            console.error('Details:', error.details);
        } else if (error.code === 'AUTHORIZATION_EXPIRED') {
            console.error('Authorization expired!');
            console.error('Details:', error.details);
        }
    } else {
        console.error('Error:', error);
    }
}
```

## Complete Example with Authorization

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';
import { createClient, Binary } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(bulletinDescriptor);

// Create client with account for authorization checking
const account = 'your-account-address';
const client = new AsyncBulletinClient(api, signer)
    .withAccount(account);

// Estimate what's needed
const data = Binary.fromBytes(new Uint8Array(5_000_000)); // 5 MB
const estimate = client.estimateAuthorization(data.asBytes().length);
console.log('Need authorization for', estimate.transactions, 'txs and', estimate.bytes, 'bytes');

// Authorize (if needed - requires sudo)
// await client.authorizeAccount(account, estimate.transactions, BigInt(estimate.bytes));

try {
    // Store - will check authorization automatically
    const result = await client.store(data).send();
    console.log('✅ Stored:', result.cid.toString());
} catch (error) {
    if (error instanceof BulletinError) {
        if (error.code === 'INSUFFICIENT_AUTHORIZATION') {
            console.error('❌ Insufficient authorization');
            console.error('   Please authorize your account first');
        } else if (error.code === 'AUTHORIZATION_EXPIRED') {
            console.error('❌ Authorization expired');
            console.error('   Please refresh your authorization');
        }
    } else {
        console.error('❌ Error:', error);
    }
}
```

## Working with Different Data Types

### Text Data

```typescript
import { Binary } from 'polkadot-api';

const data = Binary.fromText('Hello, Bulletin Chain!');
const result = await client.store(data).send();
```

### Binary Data

```typescript
// From Uint8Array
const bytes = new Uint8Array([1, 2, 3, 4, 5]);
const data = Binary.fromBytes(bytes);
const result = await client.store(data).send();
```

### File Data (Node.js)

```typescript
import { readFile } from 'fs/promises';
import { Binary } from 'polkadot-api';

const fileBuffer = await readFile('document.pdf');
const data = Binary.fromBytes(fileBuffer);
const result = await client.store(data).send();
```

### JSON Data

```typescript
import { Binary } from 'polkadot-api';

const jsonData = { message: 'Hello', timestamp: Date.now() };
const jsonString = JSON.stringify(jsonData);
const data = Binary.fromText(jsonString);
const result = await client.store(data).send();
```

## Testing Without a Node

For unit tests, use the `MockBulletinClient`:

```typescript
import { MockBulletinClient } from '@bulletin/sdk';
import { Binary } from 'polkadot-api';

// Create mock client (no blockchain required)
const client = new MockBulletinClient();

// Store data - calculates real CIDs but doesn't submit to chain
const data = Binary.fromText('Test data');
const result = await client.store(data).send();

// Verify operations performed
const ops = client.getOperations();
expect(ops).toHaveLength(1);
expect(ops[0].type).toBe('store');
```

See the [Mock Testing](../rust/mock-testing.md) guide for more details.
