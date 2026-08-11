# Authorization

This guide shows how to manage authorization using the TypeScript SDK.

> **Prerequisites**: Read [Authorization Concepts](../concepts/authorization.md) first to understand the authorization model.

## Overview

Before storing data on Bulletin Chain, accounts must be authorized. Authorization specifies:
- **transactions**: Number of store transactions allowed
- **bytes**: Total bytes allowed to store

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
  // `*_allowance` fields are the caps; `transactions`/`bytes` are consumed so far.
  // The caps gate transaction *priority*, not acceptance: going over doesn't
  // reject stores, they just lose their priority boost.
  const txBoostRemaining = auth.extent.transactions_allowance - auth.extent.transactions;
  const bytesBoostRemaining = auth.extent.bytes_allowance - auth.extent.bytes;
  console.log("Boost-tier transactions remaining:", txBoostRemaining);
  console.log("Boost-tier bytes remaining:", bytesBoostRemaining);
  console.log("Expires at block:", auth.expiration ?? "Never");
} else {
  console.log("Account not authorized");
}
```

### Check Preimage Authorization

```typescript
// Check if a specific content hash is pre-authorized.
// Fixed-size hashes are passed as 0x-prefixed hex (PAPI SizedHex).
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

Or using `AsyncBulletinClient`:

```typescript
const estimate = client.estimateAuthorization(fileSize);
```

The estimation accounts for:
- Chunking overhead (for files > 2 MiB)
- DAG-PB manifest size
- A small safety margin

## Granting Authorization

Authorization can only be granted by privileged accounts (sudo/root on testnets).

### Authorize Account

```typescript
// Only works with sudo access
const authTx = api.tx.Sudo.sudo({
  call: api.tx.TransactionStorage.authorize_account({
    who: targetAddress,
    transactions: 100,
    bytes: BigInt(100 * 1024 * 1024) // 100 MiB
  })
});

await authTx.signAndSubmit(sudoSigner);
console.log("Account authorized!");
```

### Authorize Preimage (Content Hash)

Pre-authorize a specific content hash. Useful for allowing anyone to store specific data:

```typescript
import { getContentHash, HashAlgorithm } from "@parity/bulletin-sdk";
import { Binary } from "polkadot-api";

// Calculate content hash for the data
const data = new TextEncoder().encode("Specific content to authorize");
const contentHash = await getContentHash(data, HashAlgorithm.Blake2b256);

// Authorize this specific content (fixed-size hashes are passed as hex)
const authTx = api.tx.Sudo.sudo({
  call: api.tx.TransactionStorage.authorize_preimage({
    content_hash: Binary.toHex(contentHash),
    max_size: BigInt(data.length)
  })
});

await authTx.signAndSubmit(sudoSigner);
console.log("Preimage authorized!");
```

## Using the Faucet (Testnets)

On testnets, the easiest way to get authorization is via the Faucet in the Console UI:

1. Open the Console UI
2. Connect your wallet
3. Go to **Dashboard**
4. Click the **Faucet** button
5. Confirm the transaction in your wallet

The faucet grants a default authorization (typically 10 transactions, 1 MiB).

## Manual Authorization Check

You can query the chain directly via PAPI before uploading to avoid wasted fees:

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

// Boost budget left = allowance (cap) - consumed. Exceeding it doesn't reject
// stores; they just lose their priority boost — so warn, don't block.
const availableTransactions = auth.extent.transactions_allowance - auth.extent.transactions;
const availableBytes = auth.extent.bytes_allowance - auth.extent.bytes;

if (availableTransactions < transactions || availableBytes < bytes) {
  console.warn("Boost budget exhausted - the upload will proceed at lower priority");
}

console.log("Proceeding with upload...");
```

## Authorization Expiration

Authorization can have an optional expiration block:

```typescript
const auth = await api.query.TransactionStorage.Authorizations.getValue({
  type: "Account",
  value: myAddress
});

if (auth?.expiration) {
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
import { createClient, Binary } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws";
import { bulletin } from "@polkadot-api/descriptors";
import { AsyncBulletinClient, BulletinPreparer } from "@parity/bulletin-sdk";

async function storeWithAuthCheck() {
  // Setup
  const papiClient = createClient(getWsProvider("wss://paseo-bulletin-next-rpc.polkadot.io"));
  const api = papiClient.getTypedApi(bulletin);
  const preparer = new BulletinPreparer();

  // Data to store
  const data = new TextEncoder().encode("Hello, Bulletin Chain!");

  // 1. Estimate authorization needs
  const estimate = preparer.estimateAuthorization(data.length);
  console.log("Need:", estimate.transactions, "txs,", estimate.bytes, "bytes");

  // 2. Check current authorization
  const auth = await api.query.TransactionStorage.Authorizations.getValue({
    type: "Account",
    value: myAddress
  });

  if (!auth) {
    console.log("Not authorized. Please use the Faucet.");
    return;
  }

  // Boost budget left = allowance (cap) - consumed (priority only, not a hard limit)
  const availableBytes = auth.extent.bytes_allowance - auth.extent.bytes;
  if (availableBytes < estimate.bytes) {
    console.warn("Boost budget exhausted - storing at lower priority");
  }

  // 3. Store via SDK
  const client = new AsyncBulletinClient(api, signer, papiClient.submit);
  const result = await client.store(data).send();
  console.log("Stored with CID:", result.cid.toString());
}
```

## Next Steps

- [Basic Storage](./basic-storage.md) - Store data using the SDK
- [Chunked Uploads](./chunked-uploads.md) - Handle large files
- [Renewal](./renewal.md) - Extend data retention
