# Authorization

Before storing data, you must be **authorized**. This mechanism prevents spam and ensures fair usage of chain storage.

## Who Can Authorize?

Authorization is granted by **privileged accounts**:

| Network | Authorizer | How to Request |
|---------|------------|----------------|
| **Testnets** (Paseo, Westend) | Sudo account | Use the **Faucet** in the Console UI |
| **Dotspark** | Sudo account | Use the Faucet or contact operators |
| **Mainnet** (Polkadot) | Governance (ideally) | Contact chain operators for now |

### Using the Faucet (Testnets)

The easiest way to get authorization on testnets:

1. Open the **Console UI** (you're likely already here!)
2. Connect your wallet (Polkadot.js, Talisman, etc.)
3. Navigate to **Faucet** in the menu
4. Request authorization for your account
5. The faucet will grant you a default allocation (transactions + bytes)

### For Application Developers

If you're building an application that needs authorization:

- **Testnets**: Use the faucet or run your own node with sudo access
- **Production**: Contact the chain operators or submit a governance proposal

### Running Your Own Node (Local Development)

For local development, you have sudo access:

```bash
# Start a local dev node
./polkadot-bulletin-chain --dev

# The //Alice account has sudo privileges
# Use it to authorize other accounts
```

## Types of Authorization

### 1. Account Authorization (`authorize_account`)

Authorizes a specific **account** to store data up to a limit.

- **Flexible**: The account can store *any* data
- **Best for**: Active users, applications with dynamic content

**Parameters:**
- `who`: The account to authorize
- `transactions`: Number of storage transactions allowed
- `bytes`: Total bytes allowed

### 2. Preimage Authorization (`authorize_preimage`)

Authorizes a specific **piece of data** (by hash) to be stored by anyone.

- **Restricted**: Only data matching the hash can be stored
- **Best for**: Sponsored uploads, known content

**Parameters:**
- `content_hash`: Hash of the data to authorize
- `max_size`: Maximum size of the data

## Checking Your Authorization

Query your current authorization status:

```typescript
// TypeScript (PAPI)
const auth = await api.query.TransactionStorage.Authorizations.getValue(accountId);
if (auth) {
  console.log(`Transactions remaining: ${auth.transactions}`);
  console.log(`Bytes remaining: ${auth.bytes}`);
}
```

```rust
// Rust (subxt)
let auth = api
    .storage()
    .at_latest()
    .await?
    .fetch(&bulletin::storage().transaction_storage().authorizations(account_id))
    .await?;
```

## Estimating Authorization Needs

Use the SDK to calculate how much authorization you need:

```rust
// Rust
let client = BulletinClient::new();
let (txs, bytes) = client.estimate_authorization(file_size_in_bytes);
println!("Need {} transactions and {} bytes", txs, bytes);
```

```typescript
// TypeScript
const client = new BulletinClient();
const { transactions, bytes } = client.estimateAuthorization(fileSizeInBytes);
console.log(`Need ${transactions} transactions and ${bytes} bytes`);
```

**Example Calculation** (100 MiB file with 1 MiB chunks):
- Chunks: 100
- Manifest: 1
- **Total Transactions**: 101
- **Total Bytes**: ~100 MiB + manifest overhead

## Authorization Expiry

Authorization may have an optional expiry block:
- If set, authorization becomes invalid after that block
- Unused authorization is not refunded
- Plan your uploads accordingly

## Next Steps

- [Storage Model](./storage.md) - How to store data
- [Data Retrieval](./retrieval.md) - Fetching stored data
- [Data Renewal](./renewal.md) - Extending retention
