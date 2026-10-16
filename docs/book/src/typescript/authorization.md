# Authorization

This guide shows how to manage authorization using the TypeScript SDK.

> **Prerequisites**: Read [Authorization Concepts](../concepts/authorization.md) first to understand the authorization model.

## Overview

Before storing data on Bulletin Chain, accounts must be authorized. Authorization grants:
- **transactions**: Number of store transactions
- **bytes**: Total bytes to store

On chain, an authorization's `extent` contains the granted `transactions_allowance` and `bytes_allowance`, and the used `transactions` and `bytes`. Stores beyond the allowance are still accepted, at lower priority.

## Checking Authorization

### Check Account Authorization

```typescript
import { createClient } from "polkadot-api";
import { bulletin } from "@polkadot-api/descriptors";

const client = createClient(wsProvider);
const api = client.getTypedApi(bulletin);

// Query current authorization for an account
const auth = await api.query.TransactionStorage.Authorizations.getValue({
  type: "Account",
  value: accountAddress
});

if (auth) {
  console.log("Transactions remaining:", auth.extent.transactions_allowance - auth.extent.transactions);
  console.log("Bytes remaining:", auth.extent.bytes_allowance - auth.extent.bytes);
  console.log("Expires at block:", auth.expiration);
} else {
  console.log("Account not authorized");
}
```

### Check Preimage Authorization

```typescript
// Check if a specific content hash is pre-authorized. PAPI represents a
// 32-byte hash as a 0x-prefixed hex string; a Uint8Array encodes a different key.
const contentHash = "0x...";  // Your content hash

const auth = await api.query.TransactionStorage.Authorizations.getValue({
  type: "Preimage",
  value: contentHash
});

if (auth) {
  console.log("Preimage authorized for", auth.extent.bytes_allowance, "bytes");
}
```

## Estimating Authorization Needs

Use the SDK to estimate how much authorization you need:

```typescript
import { BulletinPreparer } from "@parity/bulletin-sdk";

const preparer = new BulletinPreparer();

// For a known file size
const fileSize = 10 * 1024 * 1024; // 10 MiB
const estimate = preparer.estimateAuthorization(fileSize);

console.log("Transactions needed:", estimate.transactions);
console.log("Bytes needed:", estimate.bytes);
```

Or using `BulletinClient`:

```typescript
const estimate = client.estimateAuthorization(fileSize);
```

The estimation is offline and approximate:
- One transaction per chunk: ⌈size / `defaultChunkSize`⌉ (2 MiB by default)
- With `createManifest` (the default), one more transaction and about 50 bytes per chunk plus 1000 bytes for the manifest, even for single-chunk data that `submit` stores without a manifest

For the exact figures of an upload, use `client.estimateUpload(input)` (`transactions`, `bytes`).

## Granting Authorization

Authorization is granted by an account in the chain's `AllowedAuthorizers` set (or via sudo). Construct the client with that account as `authorizerSigner`, then use the SDK methods — `authorizeAccount` / `authorizePreimage` are signed by `authorizerSigner`, while uploads use `uploadSigner`.

### Authorize Account

```typescript
const client = new BulletinClient({
  providers: () => [getWsProvider(url)],
  uploadSigner,
  authorizerSigner, // an AllowedAuthorizers account
});

await client
  .authorizeAccount(targetAddress, 100, BigInt(100 * 1024 * 1024)) // 100 MiB
  .send();

// If the authorizer is the sudo key rather than an AllowedAuthorizer:
// await client.authorizeAccount(targetAddress, 100, ...).withSudo().send();
```

### Authorize Preimage (Content Hash)

Pre-authorize a specific content hash so anyone can store that exact data (used by the unsigned `.asUnsigned()` upload path):

```typescript
import { getContentHash, HashAlgorithm } from "@parity/bulletin-sdk";

const data = new TextEncoder().encode("Specific content to authorize");
const contentHash = await getContentHash(data, HashAlgorithm.Blake2b256);

await client.authorizePreimage(contentHash, BigInt(data.length)).send();
```

## Using the Faucet (Testnets)

On testnets, the easiest way to get authorization is via the Faucet in the Console UI:

1. Open the Console UI
2. Open the **Faucet** page
3. Enter the account (or content) to authorize and submit

The faucet signs with its own authorizer key, so no wallet confirmation is needed. Its form defaults to 100 transactions and 10 MB (editable).

## Manual Authorization Check

You can query the chain directly via PAPI before uploading, to fail fast:

```typescript
import { BulletinPreparer } from "@parity/bulletin-sdk";

const preparer = new BulletinPreparer();
const fileSize = myFile.length;

// Get estimate
const { transactions, bytes } = preparer.estimateAuthorization(fileSize);

// Query current authorization via PAPI
const auth = await api.query.TransactionStorage.Authorizations.getValue({
  type: "Account",
  value: myAddress
});

if (!auth) {
  throw new Error("Not authorized. Request authorization first.");
}

const remainingTransactions = auth.extent.transactions_allowance - auth.extent.transactions;
const remainingBytes = auth.extent.bytes_allowance - auth.extent.bytes;

if (remainingTransactions < transactions) {
  throw new Error(`Need ${transactions} transactions, have ${remainingTransactions}`);
}

if (remainingBytes < BigInt(bytes)) {
  throw new Error(`Need ${bytes} bytes, have ${remainingBytes}`);
}

console.log("Authorization sufficient, proceeding with upload...");
```

## Authorization Expiration

Every authorization has an expiration block:

```typescript
const auth = await api.query.TransactionStorage.Authorizations.getValue({
  type: "Account",
  value: myAddress
});

if (auth) {
  const currentBlock = await api.query.System.Number.getValue();
  const blocksRemaining = auth.expiration - currentBlock;

  if (blocksRemaining <= 0) {
    console.log("Authorization has expired!");
  } else {
    console.log(`Authorization expires in ${blocksRemaining} blocks`);
  }
}
```

## Complete Example

```typescript
import { getWsProvider } from "polkadot-api/ws";
import { bulletin } from "@polkadot-api/descriptors";
import { BulletinClient, blobFromBytes } from "@parity/bulletin-sdk";

async function storeWithAuthCheck() {
  const client = new BulletinClient({
    providers: () => [getWsProvider("wss://paseo-bulletin-next-rpc.polkadot.io")],
    uploadSigner: signer,
    descriptor: bulletin,
  });

  const data = new TextEncoder().encode("Hello, Bulletin Chain!");
  const src = blobFromBytes(data);

  // 1. Estimate the upload.
  const estimate = await client.estimateUpload(src);
  console.log("Need:", estimate.transactions, "txs,", estimate.bytes, "bytes");

  // 2. Check the remaining allowance (reuse the client's own connection).
  const auth = await client.api.query?.TransactionStorage.Authorizations.getValue({
    type: "Account",
    value: myAddress,
  });
  const remainingBytes = auth ? (auth.extent.bytes_allowance ?? 0n) - auth.extent.bytes : 0n;
  if (remainingBytes < estimate.bytes) {
    console.log("Allowance too low: stores beyond it get lower priority. Top up via the Faucet.");
  }

  // 3. Store. `.ensureAuthorized()` fails fast if the authorization is missing or expired.
  const { cids } = await client.submit(estimate, src).ensureAuthorized().send();
  console.log("Stored with CID:", cids[cids.length - 1].toString());
}
```

## Next Steps

- [Basic Storage](./basic-storage.md) - Store data using the SDK
- [Chunked Uploads](./chunked-uploads.md) - Handle large files
- [Renewal](./renewal.md) - Extend data retention
