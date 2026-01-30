# PAPI Integration

The SDK handles data preparation. You need `polkadot-api` to talk to the blockchain.

```typescript
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/web';
import { bulletin } from './bulletin-descriptors'; // Your chain descriptors

// Setup PAPI
const wsProvider = getWsProvider('ws://localhost:9944');
const papi = createClient(wsProvider);
const api = papi.getTypedApi(bulletin);

// ... Prepare data with SDK ...

// Submit Transaction
const tx = api.tx.TransactionStorage.store({
    data: preparedData
});

await tx.signAndSubmit(signer);
```
