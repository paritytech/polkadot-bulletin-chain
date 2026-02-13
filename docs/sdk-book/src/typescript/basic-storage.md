# Basic Storage

Store data using `AsyncBulletinClient` with PAPI.

> `store().send()` is not yet fully implemented. Authorization operations work. See the [examples](../../../../examples/) for current patterns.

## Example

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';
import { createClient, Binary } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(bulletinDescriptor);

const client = new AsyncBulletinClient(api, signer);

const data = Binary.fromText('Hello, Bulletin Chain!');
const result = await client.store(data).send();
console.log('CID:', result.cid.toString());
```

## Builder Options

```typescript
// Custom codec and hash
const result = await client
    .store(data)
    .withCodec(CidCodec.Raw)
    .withHashAlgorithm(HashAlgorithm.Blake2b256)
    .withFinalization(true)
    .send();

// With progress callback
const result = await client
    .store(data)
    .withCallback((event) => {
        if (event.type === 'chunk_completed') {
            console.log(`Chunk ${event.index + 1}/${event.total}`);
        }
    })
    .send();
```

## Authorization Checking

Set an account to enable pre-flight authorization checking:

```typescript
const client = new AsyncBulletinClient(api, signer)
    .withAccount(account);

// Fails with INSUFFICIENT_AUTHORIZATION or AUTHORIZATION_EXPIRED if not authorized
const result = await client.store(data).send();
```

To disable:

```typescript
const client = new AsyncBulletinClient(api, signer, {
    checkAuthorizationBeforeUpload: false,
});
```

## Testing

```typescript
import { MockBulletinClient } from '@bulletin/sdk';

const client = new MockBulletinClient();
const result = await client.store(data).send();

const ops = client.getOperations();
expect(ops).toHaveLength(1);
```
