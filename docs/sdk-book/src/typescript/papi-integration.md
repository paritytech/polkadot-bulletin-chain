# PAPI Integration

The SDK requires a PAPI client and signer.

## Setup

```bash
npm install polkadot-api @polkadot-labs/hdkd @polkadot-labs/hdkd-helpers
```

## Connection

```typescript
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';
import { AsyncBulletinClient } from '@bulletin/sdk';

const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(bulletinDescriptor);

const client = new AsyncBulletinClient(api, signer);
```

## Chain Descriptors

Generate chain descriptors for type-safe interaction:

```bash
npx papi add wss://bulletin-rpc.polkadot.io -n bulletin
```

```typescript
import { bulletin } from 'polkadot-api/descriptors';
const api = papiClient.getTypedApi(bulletin);
```

## Signers

### Dev Accounts

```typescript
import { sr25519CreateDerive } from '@polkadot-labs/hdkd';
import { getPolkadotSigner } from 'polkadot-api/signer';
import { DEV_PHRASE } from '@polkadot-labs/hdkd-helpers';

const keyring = sr25519CreateDerive(DEV_PHRASE);
const signer = getPolkadotSigner(keyring.derive("//Alice"), "Alice", 42);
```

### Production

```typescript
const keyring = sr25519CreateDerive("your mnemonic phrase here");
const signer = getPolkadotSigner(keyring.derive("//0"), "My Account", 42);
```

### Browser Wallet

```typescript
import { connectInjectedExtension } from 'polkadot-api/pjs-signer';

const extension = await connectInjectedExtension('polkadot-js');
const accounts = extension.getAccounts();
const client = new AsyncBulletinClient(api, accounts[0].polkadotSigner);
```

## Cleanup

```typescript
await papiClient.destroy();
```
