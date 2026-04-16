# PAPI Integration

The TypeScript SDK follows a **Bring Your Own Client (BYOC)** pattern - you provide a configured PAPI client and signer. This design enables:

- **Light client support** via smoldot (no RPC endpoint required)
- **Connection reuse** - share one client across SDK and other code
- **Browser wallet integration** - use Talisman, SubWallet, etc.
- **Custom transports** - HTTP, WebSocket, or any PAPI-compatible provider

## Setup

First, install the required dependencies:

```bash
npm install polkadot-api @polkadot-labs/hdkd @polkadot-labs/hdkd-helpers
```

## Connection Options

### WebSocket Connection (Traditional)

```typescript
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws';
import { AsyncBulletinClient } from '@parity/bulletin-sdk';
import { sr25519CreateDerive } from '@polkadot-labs/hdkd';
import { getPolkadotSigner } from 'polkadot-api/signer';
import { DEV_MINI_SECRET } from '@polkadot-labs/hdkd-helpers';

// 1. Setup WebSocket connection
const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);

// 2. Get typed API (you'll need chain descriptors)
const api = papiClient.getTypedApi(bulletinDescriptor);

// 3. Create signer
const derive = sr25519CreateDerive(DEV_MINI_SECRET);
const aliceKeyPair = derive("//Alice");
const signer = getPolkadotSigner(
    aliceKeyPair.publicKey,
    "Sr25519",
    aliceKeyPair.sign,
);

// 4. Create SDK client with PAPI client, signer, and submit function
const client = new AsyncBulletinClient(api, signer, papiClient.submit);

// 5. Use the client
const data = new TextEncoder().encode('Hello, Bulletin!');
const result = await client.store(data).send();

console.log('Stored with CID:', result.cid.toString());
```

### Light Client Connection (Smoldot)

For trustless, decentralized connections without relying on RPC endpoints:

```typescript
import { createClient } from 'polkadot-api';
import { getSmProvider } from 'polkadot-api/sm-provider';
import { startFromWorker } from 'polkadot-api/smoldot/from-worker';
import SmWorker from 'polkadot-api/smoldot/worker?worker';
import { AsyncBulletinClient } from '@parity/bulletin-sdk';

// 1. Start smoldot in a web worker
const smoldot = startFromWorker(new SmWorker());

// 2. Add the Bulletin chain spec
const bulletinChainSpec = await fetch('/chainspecs/bulletin.json').then(r => r.text());
const chain = await smoldot.addChain({ chainSpec: bulletinChainSpec });

// 3. Create PAPI client with smoldot provider
const smProvider = getSmProvider(chain);
const papiClient = createClient(smProvider);

// 4. Get typed API and create SDK client
const api = papiClient.getTypedApi(bulletinDescriptor);
const client = new AsyncBulletinClient(api, signer, papiClient.submit);

// Use normally - SDK doesn't know or care about the transport!
const result = await client.store(data).send();
```

**Benefits of light clients:**
- No trusted RPC endpoint required
- Verifies chain state cryptographically
- Works offline after initial sync
- Better for user privacy

### Relay Chain + Parachain (Light Client)

For parachains, connect through the relay chain:

```typescript
import { createClient } from 'polkadot-api';
import { getSmProvider } from 'polkadot-api/sm-provider';
import { startFromWorker } from 'polkadot-api/smoldot/from-worker';
import SmWorker from 'polkadot-api/smoldot/worker?worker';

// Start smoldot
const smoldot = startFromWorker(new SmWorker());

// Add relay chain first
const relayChainSpec = await fetch('/chainspecs/polkadot.json').then(r => r.text());
const relayChain = await smoldot.addChain({ chainSpec: relayChainSpec });

// Add Bulletin as parachain
const bulletinChainSpec = await fetch('/chainspecs/bulletin-parachain.json').then(r => r.text());
const bulletinChain = await smoldot.addChain({
    chainSpec: bulletinChainSpec,
    potentialRelayChains: [relayChain],
});

// Create client
const papiClient = createClient(getSmProvider(bulletinChain));
const api = papiClient.getTypedApi(bulletinDescriptor);
const client = new AsyncBulletinClient(api, signer, papiClient.submit);
```

## Connection Reuse

The SDK accepts an existing PAPI client, so you can share one connection across your entire application:

```typescript
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws';
import { AsyncBulletinClient } from '@parity/bulletin-sdk';

// Create ONE shared PAPI client for your whole app
const wsProvider = getWsProvider('wss://bulletin-rpc.polkadot.io');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(bulletinDescriptor);

// SDK uses the shared client
const bulletinClient = new AsyncBulletinClient(api, signer, papiClient.submit);

// Your other code also uses the same client
const blockNumber = await api.query.System.Number.getValue();
const events = await api.query.System.Events.getValue();

// Both SDK operations and your queries share one WebSocket connection!
const result = await bulletinClient.store(data).send();
```

**Why connection sharing matters:**
- Avoids hitting connection limits on RPC endpoints
- Reduces memory usage
- Single subscription for block updates
- Consistent chain state across your app

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
import { DEV_MINI_SECRET } from '@polkadot-labs/hdkd-helpers';

const derive = sr25519CreateDerive(DEV_MINI_SECRET);

// Alice
const aliceKeyPair = derive("//Alice");
const aliceSigner = getPolkadotSigner(
    aliceKeyPair.publicKey,
    "Sr25519",
    aliceKeyPair.sign,
);

// Bob
const bobKeyPair = derive("//Bob");
const bobSigner = getPolkadotSigner(
    bobKeyPair.publicKey,
    "Sr25519",
    bobKeyPair.sign,
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
const client = new AsyncBulletinClient(api, accounts[0].polkadotSigner, papiClient.submit);
```

## Multiple Accounts

When you need to use different accounts (e.g., Alice for authorization, Bob for storage), create separate clients:

```typescript
// Client for Alice (sudo account)
const aliceClient = new AsyncBulletinClient(api, aliceSigner, papiClient.submit);

// Client for Bob (regular user)
const bobClient = new AsyncBulletinClient(api, bobSigner, papiClient.submit);

// Alice authorizes Bob
const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty";
await aliceClient.authorizeAccount(bobAddress, 100, BigInt(10_000_000)).send();

// Bob stores data
const result = await bobClient.store(data).send();
```

## Direct Transaction Preparation

For advanced use cases, use `BulletinPreparer` for offline CID calculation without chain interaction.

**Small data** (< 2 MiB) — single transaction:

```typescript
import { BulletinPreparer } from '@parity/bulletin-sdk';

const preparer = new BulletinPreparer();
const prepared = await preparer.prepareStore(smallData);

console.log('CID:', prepared.cid.toString());

const tx = api.tx.TransactionStorage.store({
    data: prepared.data
});
await tx.signAndSubmit(signer);
```

**Large data** — use `prepareStoreChunked` and submit each chunk separately:

```typescript
const prepared = await preparer.prepareStoreChunked(largeData);

for (const chunk of prepared.chunks) {
    const tx = api.tx.TransactionStorage.store({
        data: chunk.data
    });
    await tx.signAndSubmit(signer);
}

if (prepared.manifest) {
    const tx = api.tx.TransactionStorage.store({
        data: prepared.manifest.data
    });
    await tx.signAndSubmit(signer);
    console.log('Manifest CID:', prepared.manifest.cid.toString());
}
```

> For most use cases, prefer `AsyncBulletinClient.store()` which handles chunking, submission, and progress tracking automatically.

## Error Handling

Handle blockchain errors properly:

```typescript
import { BulletinError } from '@parity/bulletin-sdk';

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
const client = new AsyncBulletinClient(api, signer, papiClient.submit, {
    defaultChunkSize: 1024 * 1024, // 1 MiB chunks
    createManifest: true, // Create DAG-PB manifest
    chunkingThreshold: 2 * 1024 * 1024, // Auto-chunk files > 2 MiB
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
