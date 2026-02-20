# PAPI Integration

The TypeScript SDK is tightly coupled to Polkadot API (PAPI) for blockchain interaction. You must provide a configured PAPI client and signer when creating the SDK client.

## Setup

First, install the required dependencies:

```bash
npm install polkadot-api @polkadot-labs/hdkd @polkadot-labs/hdkd-helpers
```

## Basic Connection

```typescript
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';
import { AsyncBulletinClient } from '@bulletin/sdk';
import { sr25519CreateDerive } from '@polkadot-labs/hdkd';
import { getPolkadotSigner } from 'polkadot-api/signer';
import { DEV_PHRASE } from '@polkadot-labs/hdkd-helpers';

// 1. Setup WebSocket connection
const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);

// 2. Get typed API (you'll need chain descriptors)
const api = papiClient.getTypedApi(bulletinDescriptor);

// 3. Create signer
const keyring = sr25519CreateDerive(DEV_PHRASE);
const signer = getPolkadotSigner(
    keyring.derive("//Alice"),
    "Alice",
    42 // Bulletin chain ID
);

// 4. Create SDK client with PAPI client and signer
const client = new AsyncBulletinClient(api, signer);

// 5. Use the client
const data = new TextEncoder().encode('Hello, Bulletin!');
const result = await client.store(data).send();

console.log('Stored with CID:', result.cid.toString());
```

## Chain Descriptors

You'll need to generate chain descriptors for your Bulletin Chain instance. These provide type information to PAPI:

```bash
npx papi add wss://bulletin-rpc.polkadot.io -n bulletin
```

This creates a `bulletin` descriptor you can import:

```typescript
import { bulletin } from 'polkadot-api/descriptors';

const api = papiClient.getTypedApi(bulletin);
```

## Using Different Signers

### Development Accounts

For testing, use dev accounts:

```typescript
import { sr25519CreateDerive } from '@polkadot-labs/hdkd';
import { getPolkadotSigner } from 'polkadot-api/signer';
import { DEV_PHRASE } from '@polkadot-labs/hdkd-helpers';

const keyring = sr25519CreateDerive(DEV_PHRASE);

// Alice
const aliceSigner = getPolkadotSigner(
    keyring.derive("//Alice"),
    "Alice",
    42
);

// Bob
const bobSigner = getPolkadotSigner(
    keyring.derive("//Bob"),
    "Bob",
    42
);
```

### Production Accounts

For production, use a seed phrase or mnemonic:

```typescript
import { sr25519CreateDerive } from '@polkadot-labs/hdkd';
import { getPolkadotSigner } from 'polkadot-api/signer';

const keyring = sr25519CreateDerive(
    "bottom drive obey lake curtain smoke basket hold race lonely fit walk"
);

const signer = getPolkadotSigner(
    keyring.derive("//0"), // Derive first account
    "My Account",
    42 // Bulletin chain ID
);
```

### Browser Wallet Extension

For browser applications with wallet extensions:

```typescript
import { getInjectedExtensions, connectInjectedExtension } from 'polkadot-api/pjs-signer';

// Get available extensions
const extensions = getInjectedExtensions();

// Connect to an extension (e.g., Talisman, Polkadot.js)
const extension = await connectInjectedExtension('polkadot-js');

// Get accounts
const accounts = extension.getAccounts();

// Create client with first account
const client = new AsyncBulletinClient(api, accounts[0].polkadotSigner);
```

## Multiple Accounts

When you need to use different accounts (e.g., Alice for authorization, Bob for storage), create separate clients:

```typescript
// Client for Alice (sudo account)
const aliceClient = new AsyncBulletinClient(api, aliceSigner);

// Client for Bob (regular user)
const bobClient = new AsyncBulletinClient(api, bobSigner);

// Alice authorizes Bob
const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty";
await aliceClient.authorizeAccount(bobAddress, 100, BigInt(10_000_000));

// Bob stores data
const result = await bobClient.store(data).send();
```

## Direct Transaction Submission

The SDK handles transaction submission internally. However, if you need to submit transactions manually (e.g., for custom batching), you can use PAPI directly:

```typescript
// Prepare data with core SDK (BulletinClient)
import { BulletinClient } from '@bulletin/sdk';

const prepClient = new BulletinClient({ endpoint: 'ws://localhost:9944' });
const operation = await prepClient.prepareStore(data);

// Submit via PAPI
const tx = api.tx.TransactionStorage.store({
    data: operation.data
});

const result = await tx.signAndSubmit(signer);
const finalized = await result.waitFor('finalized');

console.log('Block hash:', finalized.blockHash);
```

## Error Handling

Handle blockchain errors properly:

```typescript
import { BulletinError } from '@bulletin/sdk';

try {
    const result = await client.store(data).send();
    console.log('Success:', result.cid.toString());
} catch (error) {
    if (error instanceof BulletinError) {
        console.error('Bulletin error:', error.code);
        console.error('Message:', error.message);
        console.error('Details:', error.cause);
    } else {
        console.error('Unexpected error:', error);
    }
}
```

## Configuration Options

Customize client behavior:

```typescript
const client = new AsyncBulletinClient(api, signer, {
    defaultChunkSize: 1024 * 1024, // 1 MiB chunks
    maxParallel: 8, // Upload 8 chunks in parallel
    createManifest: true, // Create DAG-PB manifest
    chunkingThreshold: 2 * 1024 * 1024, // Auto-chunk files > 2 MiB
    checkAuthorizationBeforeUpload: true, // Validate auth before upload
});
```

## Cleanup

Always clean up connections when done:

```typescript
// Store data
const result = await client.store(data).send();

// Cleanup
await papiClient.destroy();
```
