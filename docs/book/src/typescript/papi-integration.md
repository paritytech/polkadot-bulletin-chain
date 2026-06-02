# PAPI Integration

The SDK owns its PAPI connection. You give the constructor a **providers factory** and a signer; the SDK builds the `PolkadotClient`, derives the typed API, and tears it all down on `destroy()`. The factory is called once per upload (and on each retry), so dead WebSockets are replaced with fresh ones.

```typescript
providers: () => JsonRpcProvider[]
```

`providers()[0]` drives the chainHead subscription; every provider in the array receives broadcast transactions. A factory is **required** for any signed or unsigned upload.

## Setup

```bash
npm install polkadot-api @polkadot-labs/hdkd @polkadot-labs/hdkd-helpers
```

## WebSocket

```typescript
import { BulletinClient } from '@parity/bulletin-sdk';
import { getWsProvider } from 'polkadot-api/ws-provider/node';
import { sr25519CreateDerive } from '@polkadot-labs/hdkd';
import { getPolkadotSigner } from 'polkadot-api/signer';
import { DEV_MINI_SECRET } from '@polkadot-labs/hdkd-helpers';
import { bulletin } from '@polkadot-api/descriptors';

const kp = sr25519CreateDerive(DEV_MINI_SECRET)('//Alice');
const signer = getPolkadotSigner(kp.publicKey, 'Sr25519', kp.sign);

const client = new BulletinClient({
  providers: () => [getWsProvider('ws://localhost:9944')],
  uploadSigner: signer,
  descriptor: bulletin, // optional; omit to use getUnsafeApi()
});
```

## Light Client (Smoldot)

Return a smoldot provider from the factory instead — nothing else changes:

```typescript
import { getSmProvider } from 'polkadot-api/sm-provider';
import { startFromWorker } from 'polkadot-api/smoldot/from-worker';
import SmWorker from 'polkadot-api/smoldot/worker?worker';

const smoldot = startFromWorker(new SmWorker());
const chain = await smoldot.addChain({
  chainSpec: await fetch('/chainspecs/bulletin.json').then((r) => r.text()),
});

const client = new BulletinClient({
  providers: () => [getSmProvider(chain)],
  uploadSigner: signer,
  descriptor: bulletin,
});
```

For a parachain, add the relay chain first and pass it as a potential relay:

```typescript
const relay = await smoldot.addChain({ chainSpec: relaySpec });
const chain = await smoldot.addChain({
  chainSpec: bulletinParaSpec,
  potentialRelayChains: [relay],
});
```

## Chain Descriptors

Generate descriptors with `papi` for compile-time chain types:

```bash
npx papi add wss://bulletin-rpc.polkadot.io -n bulletin
```

```typescript
import { bulletin } from '@polkadot-api/descriptors';
// pass `descriptor: bulletin` to the constructor
```

Omit `descriptor` to fall back to `getUnsafeApi()` — it works at runtime but loses the types.

## Sharing the Connection

The SDK exposes its typed API as `client.api`, so your own queries reuse the same connection:

```typescript
const blockNumber = await client.api.query.System.Number.getValue();
const auth = await client.api.query.TransactionStorage.Authorizations.getValue({
  type: 'Account',
  value: address,
});
```

## Signers

A client has up to two signers, both optional:

- `uploadSigner` — signs `store` extrinsics. Omit it for an unsigned-only client (`.asUnsigned()`); signed paths then throw `UNSUPPORTED_OPERATION`.
- `authorizerSigner` — signs authorization extrinsics (`authorizeAccount`, `authorizePreimage`, refresh). Required to call those.

```typescript
// One client where Eve (an authorizer) grants quota and Alice uploads.
const client = new BulletinClient({
  providers: () => [getWsProvider(url)],
  uploadSigner: aliceSigner,
  authorizerSigner: eveSigner,
});

await client.authorizeAccount(aliceAddress, 100, 100n * 1024n * 1024n).send();
const { cids } = await client.submit(await client.estimateUpload(src), src).send();
```

### Browser Wallet

```typescript
import { connectInjectedExtension } from 'polkadot-api/pjs-signer';

const extension = await connectInjectedExtension('polkadot-js');
const account = extension.getAccounts()[0];

const client = new BulletinClient({
  providers: () => [getWsProvider(url)],
  uploadSigner: account.polkadotSigner,
});
```

## Offline Preparation

`BulletinPreparer` computes CIDs and chunk plans without a connection — useful for previews or custom submission. `estimateUpload` uses it internally; reach for it directly only when you submit the raw extrinsics yourself.

```typescript
import { BulletinPreparer } from '@parity/bulletin-sdk';

const preparer = new BulletinPreparer();
const { cid } = await preparer.prepareStore(smallData);
```

## Configuration

```typescript
const client = new BulletinClient({
  providers: () => [getWsProvider(url)],
  uploadSigner: signer,
  defaultChunkSize: 1024 * 1024,        // 1 MiB (max 2 MiB)
  chunkingThreshold: 2 * 1024 * 1024,   // single-tx limit
  txTimeout: 420_000,                   // per-tx inclusion timeout (ms)
});
```

## Cleanup

`destroy()` tears down the PAPI client the SDK built:

```typescript
await client.destroy();
```
