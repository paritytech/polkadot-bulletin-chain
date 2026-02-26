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
  console.log("Transactions remaining:", auth.extent.transactions);
  console.log("Bytes remaining:", auth.extent.bytes);
  console.log("Expires at block:", auth.expiration ?? "Never");
} else {
  console.log("Account not authorized");
}
```

### Check Preimage Authorization

```typescript
import { Binary } from "polkadot-api";

// Check if a specific content hash is pre-authorized
const contentHash = Binary.fromHex("0x...");  // Your content hash

const auth = await api.query.TransactionStorage.Authorizations.getValue({
  type: "Preimage",
  value: contentHash
});

if (auth) {
  console.log("Preimage authorized for", auth.extent.bytes, "bytes");
}
```

## Estimating Authorization Needs

Use the SDK to estimate how much authorization you need:

```typescript
import { BulletinClient } from "@bulletin/sdk";

const client = new BulletinClient();

// For a known file size
const fileSize = 10 * 1024 * 1024; // 10 MiB
const estimate = client.estimateAuthorization(fileSize);

console.log("Transactions needed:", estimate.transactions);
console.log("Bytes needed:", estimate.bytes);
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
import { calculateCid, getContentHash, HashAlgorithm, CidCodec } from "@bulletin/sdk";

// Calculate content hash for the data
const data = new TextEncoder().encode("Specific content to authorize");
const contentHash = await getContentHash(data, HashAlgorithm.Blake2b256);

// Authorize this specific content
const authTx = api.tx.Sudo.sudo({
  call: api.tx.TransactionStorage.authorize_preimage({
    content_hash: Binary.fromBytes(contentHash),
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

## Pre-flight Authorization Checking

The SDK can automatically check authorization before uploading:

```typescript
import { AsyncBulletinClient } from "@bulletin/sdk";

const client = new AsyncBulletinClient(api, signer);

// Enable pre-flight checking
const result = await client
  .store(data)
  .withAccount(myAddress)  // Enable authorization checking
  .send();

// If authorization is insufficient, throws InsufficientAuthorization error
// BEFORE submitting to the chain (saves transaction fees!)
```

### Manual Pre-flight Check

```typescript
import { BulletinClient } from "@bulletin/sdk";

const client = new BulletinClient();
const fileSize = myFile.length;

// Get estimate
const { transactions, bytes } = client.estimateAuthorization(fileSize);

// Query current authorization
const auth = await api.query.TransactionStorage.Authorizations.getValue({
  type: "Account",
  value: myAddress
});

if (!auth) {
  throw new Error("Not authorized. Request authorization first.");
}

if (auth.extent.transactions < transactions) {
  throw new Error(`Need ${transactions} transactions, have ${auth.extent.transactions}`);
}

if (auth.extent.bytes < bytes) {
  throw new Error(`Need ${bytes} bytes, have ${auth.extent.bytes}`);
}

console.log("Authorization sufficient, proceeding with upload...");
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
import { getWsProvider } from "polkadot-api/ws-provider/node";
import { bulletin } from "@polkadot-api/descriptors";
import { BulletinClient, calculateCid, CidCodec, HashAlgorithm } from "@bulletin/sdk";

async function storeWithAuthCheck() {
  // Setup
  const client = createClient(getWsProvider("wss://paseo-bulletin-rpc.polkadot.io"));
  const api = client.getTypedApi(bulletin);
  const sdkClient = new BulletinClient();

  // Data to store
  const data = new TextEncoder().encode("Hello, Bulletin Chain!");

  // 1. Estimate authorization needs
  const estimate = sdkClient.estimateAuthorization(data.length);
  console.log("Need:", estimate.transactions, "txs,", estimate.bytes, "bytes");

  // 2. Check current authorization
  const auth = await api.query.TransactionStorage.Authorizations.getValue({
    type: "Account",
    value: myAddress
  });

  if (!auth || auth.extent.bytes < estimate.bytes) {
    console.log("Insufficient authorization. Please use the Faucet.");
    return;
  }

  // 3. Calculate CID
  const cid = await calculateCid(data, CidCodec.Raw, HashAlgorithm.Blake2b256);
  console.log("CID will be:", cid.toString());

  // 4. Submit transaction
  const tx = api.tx.TransactionStorage.store({
    data: Binary.fromBytes(data),
    cid_config: { codec: 0x55, hashing: "Blake2b256" }
  });

  const result = await tx.signAndSubmit(signer);
  console.log("Stored in block:", result.block.number);
}
```

## Next Steps

- [Basic Storage](./basic-storage.md) - Store data using the SDK
- [Chunked Uploads](./chunked-uploads.md) - Handle large files
- [Renewal](./renewal.md) - Extend data retention
