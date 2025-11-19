# Using PAPI with Polkadot Bulletin Chain

## Setup

1. **Install dependencies:**
```bash
npm install polkadot-api @polkadot-api/pjs-signer @polkadot/keyring @polkadot/util-crypto multiformats ipfs-http-client
```

2. **Generate PAPI descriptors:**

Make sure your Bulletin node is running, then generate the type-safe descriptors:

```bash
npx papi add -w ws://localhost:10000 bulletin
```

This will:
- Connect to your local bulletin chain
- Download the metadata
- Generate TypeScript types in `.papi/descriptors/bulletin.ts`
- Create metadata files in `.papi/metadata/bulletin.scale`

Alternatively, if you have issues connecting, you can use the pre-generated metadata from:
https://github.com/paritytech/product-infrastructure/blob/main/apps/bulletin-example/.papi/metadata/bulletin.scale

3. **Run the example:**
```bash
node authorize_and_store_papi.js
```

## Key Differences from @polkadot/api

### 1. Client Creation
**Old (@polkadot/api):**
```javascript
const ws = new WsProvider('ws://localhost:10000');
const api = await ApiPromise.create({ provider: ws });
```

**New (PAPI):**
```javascript
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

const wsProvider = getWsProvider('ws://localhost:10000');
const client = createClient(wsProvider);
const typedApi = client.getTypedApi(bulletin);
```

### 2. Transactions
**Old:**
```javascript
const tx = api.tx.transactionStorage.store(data);
const result = await tx.signAndSend(pair);
```

**New:**
```javascript
const tx = typedApi.tx.TransactionStorage.store({ data });
const result = await tx.signAndSubmit(pair);
```

### 3. Type Safety
PAPI provides full TypeScript type safety based on your chain's metadata:
- Transaction parameters are type-checked
- Query results have proper types
- Auto-completion in IDEs

### 4. Signing
PAPI uses a different signing interface. The `@polkadot-api/pjs-signer` package bridges between `@polkadot/keyring` and PAPI:

```javascript
import { getPolkadotSignerFromPjs } from '@polkadot-api/pjs-signer';
import { Keyring } from '@polkadot/keyring';

const keyring = new Keyring({ type: 'sr25519' });
const account = keyring.addFromUri('//Alice');

// Create PAPI-compatible signer (simple!)
const signer = getPolkadotSignerFromPjs(account);
```

## Benefits of PAPI

1. **Type Safety**: Full TypeScript support with generated types
2. **Light Client Support**: Can use smoldot for light client connections
3. **Better Performance**: More efficient serialization/deserialization
4. **Modern API**: Cleaner, more intuitive API design
5. **Better Developer Experience**: Auto-completion and type checking

## Troubleshooting

### Error: Cannot find module '@polkadot-api/descriptors'
Run: `npx papi add -w ws://localhost:10000 bulletin`

### Connection issues
Make sure your bulletin chain node is running on ws://localhost:10000

### Metadata errors
If metadata changes, regenerate descriptors: `npx papi`

