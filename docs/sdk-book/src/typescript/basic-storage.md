# Basic Storage

This guide shows how to store data using the `AsyncBulletinClient` with transaction submitters.

## Quick Start

The `store()` method automatically handles both small and large files:

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

// 1. Connect to Bulletin Chain
const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(/* your chain descriptors */);

// 2. Create submitter and client
const submitter = new PAPITransactionSubmitter(api, signer);
const client = new AsyncBulletinClient(submitter);

// 3. Store data (automatically chunks if > 2 MiB)
const data = new TextEncoder().encode('Hello, Bulletin Chain!');
const result = await client.store(data);

console.log('✅ Stored successfully!');
console.log('   CID:', result.cid.toString());
console.log('   Size:', result.size, 'bytes');
```

## Step-by-Step Explanation

### 1. Setup Connection

First, create a transaction submitter. The submitter handles all blockchain communication:

```typescript
import { PAPITransactionSubmitter } from '@bulletin/sdk';
import { createClient } from 'polkadot-api';

// Connect to chain
const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(/* descriptors */);

// Create submitter
const submitter = new PAPITransactionSubmitter(api, signer);
```

### 2. Create Client

Wrap the submitter with `AsyncBulletinClient`:

```typescript
const client = new AsyncBulletinClient(submitter);
```

### 3. Store Data

The `store()` method automatically handles everything:
- Validates data size
- Checks authorization (if configured)
- Automatically chunks large files (> 2 MiB by default)
- Calculates CID(s)
- Submits transaction(s)
- Waits for finalization

```typescript
// For small files (< 2 MiB): single transaction
// For large files (> 2 MiB): automatic chunking
const result = await client.store(data);

// With progress tracking for large files
const result = await client.store(data, undefined, (event) => {
    if (event.type === 'chunk_completed') {
        console.log(`Chunk ${event.index + 1}/${event.total} uploaded`);
    } else if (event.type === 'completed') {
        console.log('Upload complete!');
    }
});
```

### 4. Handle Result

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

// 1. Create client with your account
const submitter = new PAPITransactionSubmitter(api, signer);
const account = 'your-account-address';

const client = new AsyncBulletinClient(submitter)
    .withAccount(account);  // Set the account for auth checking

// 2. Upload - authorization is checked automatically
const data = new TextEncoder().encode('Hello, Bulletin!');
const result = await client.store(data);
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
const client = new AsyncBulletinClient(submitter, {
    checkAuthorizationBeforeUpload: false,  // Disable checking
}).withAccount(account);
```

### Error Example

```typescript
try {
    const result = await client.store(data);
    console.log('Success!');
} catch (error) {
    if (error.code === 'INSUFFICIENT_AUTHORIZATION') {
        console.error('Need more authorization!');
        console.error('Details:', error.cause);
    } else if (error.code === 'AUTHORIZATION_EXPIRED') {
        console.error('Authorization expired!');
        console.error('Details:', error.cause);
    } else {
        console.error('Error:', error);
    }
}
```

## Complete Example with Authorization

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(/* descriptors */);

// Create client with account for authorization checking
const submitter = new PAPITransactionSubmitter(api, signer);
const account = 'your-account-address';
const client = new AsyncBulletinClient(submitter)
    .withAccount(account);

// Estimate what's needed
const data = new Uint8Array(5_000_000); // 5 MB
const estimate = client.estimateAuthorization(data.length);
console.log('Need authorization for', estimate.transactions, 'txs and', estimate.bytes, 'bytes');

// Authorize (if needed - requires sudo)
// await client.authorizeAccount(account, estimate.transactions, BigInt(estimate.bytes));

try {
    // Store - will check authorization automatically
    const result = await client.store(data);
    console.log('✅ Stored:', result.cid.toString());
} catch (error) {
    if (error.code === 'INSUFFICIENT_AUTHORIZATION') {
        console.error('❌ Insufficient authorization');
        console.error('   Please authorize your account first');
    } else if (error.code === 'AUTHORIZATION_EXPIRED') {
        console.error('❌ Authorization expired');
        console.error('   Please refresh your authorization');
    } else {
        console.error('❌ Error:', error.message);
    }
}
```

## Testing Without a Node

For unit tests, implement a mock submitter:

```typescript
class MockSubmitter implements TransactionSubmitter {
    async submitStore(data: Uint8Array) {
        return {
            blockHash: '0xmock',
            txHash: '0xmock',
            blockNumber: 1,
        };
    }

    // Implement other required methods...

    // Optional: implement query methods for testing authorization
    async queryAccountAuthorization(who: string) {
        return {
            scope: AuthorizationScope.Account,
            transactions: 100,
            maxSize: BigInt(10_000_000),
            expiresAt: undefined,
        };
    }
}

// Use in tests
const submitter = new MockSubmitter();
const client = new AsyncBulletinClient(submitter);
const result = await client.store(testData);
```
