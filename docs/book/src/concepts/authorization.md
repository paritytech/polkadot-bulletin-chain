# Authorization

Before storing data, you must be **authorized**. This mechanism prevents spam and ensures fair usage of chain storage.

## Who Can Authorize?

Authorization is granted by **privileged accounts**:

| Network | Authorizer | How to Request |
|---------|------------|----------------|
| **Testnets** (Paseo, Westend) | Registered authorizer accounts (the faucet signs as `//Eve`) or the [sudo account](#finding-the-sudo-account) | Use the **Faucet** in the Console UI |
| **Mainnet** (Polkadot) | Governance (ideally) | Contact chain operators for now |

### Finding the Sudo Account

The sudo account for each network can be queried on-chain:

```typescript
// Query the current sudo key
const sudoKey = await api.query.Sudo.Key.getValue();
console.log("Sudo account:", sudoKey);
```

### Using the Faucet (Testnets)

The easiest way to get authorization on testnets:

1. Open the **Console UI** (you're likely already here!)
2. Navigate to **Faucet** in the menu
3. Enter the account to authorize (or the content to pre-authorize)
4. Submit: the faucet signs with its own authorizer key (`//Eve`), so no wallet confirmation is needed. The form defaults to 100 transactions and 10 MB

### For Application Developers

If you're building an application that needs authorization:

- **Testnets**: Use the faucet or run your own node with sudo access
- **Production**: Contact the chain operators or submit a governance proposal

### Running Your Own Node (Local Development)

For local development, you have sudo access:

```bash
# Build the Bulletin chain spec, then start a local dev node with it
just binaries-polkadot
just chain-spec westend
$(just binaries-polkadot)/polkadot-omni-node --chain ./zombienet/bulletin-westend-spec.json --dev

# The //Alice account has sudo privileges; //Eve is a registered authorizer
# Use them to authorize other accounts
```

## Types of Authorization

### 1. Account Authorization (`authorize_account`)

Authorizes a specific **account** to store data with an allowance of bytes and transactions.

- **Flexible**: The account can store *any* data
- **Best for**: Active users, applications with dynamic content
- **Soft limit for stores**: stores within the allowance get a priority boost; stores beyond it still go through at lower priority. Renewals are hard-capped by `bytes`
- Calling it again before the authorization expires adds to the allowance without extending the expiry

**Parameters:**
- `who`: The account to authorize
- `transactions`: Number of boost-tier storage transactions
- `bytes`: Total boost-tier bytes

### 2. Preimage Authorization (`authorize_preimage`)

Authorizes a specific **piece of data** (by hash) to be stored by anyone.

- **Restricted**: Only data matching the hash can be stored
- **Best for**: Sponsored uploads, known content

**Parameters:**
- `content_hash`: Hash of the data to authorize
- `max_size`: Maximum size of the data

## Checking Your Authorization

Query your current authorization status:

The `Authorizations` map is keyed by an `AuthorizationScope` (`Account(accountId)` or `Preimage(contentHash)`). The stored value carries an `extent` with the consumed counters (`transactions`, `bytes`) and the granted caps (`transactions_allowance`, `bytes_allowance`).

```typescript
// TypeScript (PAPI) - key is an AuthorizationScope enum
const auth = await api.query.TransactionStorage.Authorizations.getValue({
  type: "Account",
  value: accountId,
});
if (auth) {
  const { transactions, transactions_allowance, bytes, bytes_allowance } = auth.extent;
  console.log(`Transactions: ${transactions} / ${transactions_allowance}`);
  console.log(`Bytes: ${bytes} / ${bytes_allowance}`);
}
```

```rust
// Rust (subxt)
use bulletin::runtime_types::pallet_bulletin_transaction_storage::types::AuthorizationScope;

let scope = AuthorizationScope::Account(account_id);
let auth = api
    .storage()
    .at_latest()
    .await?
    .fetch(&bulletin::storage().transaction_storage().authorizations(scope))
    .await?;
if let Some(auth) = auth {
    println!("Transactions: {}/{}", auth.extent.transactions, auth.extent.transactions_allowance);
    println!("Bytes: {}/{}", auth.extent.bytes, auth.extent.bytes_allowance);
}
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
import { estimateAuthorization } from "@parity/bulletin-sdk";

const chunkSize = 2 * 1024 * 1024; // 2 MiB
const { transactions, bytes } = estimateAuthorization(fileSizeInBytes, chunkSize, true);
console.log(`Need ${transactions} transactions and ${bytes} bytes`);
```

**Example Calculation** (100 MiB file with 2 MiB chunks):
- Chunks: 50
- Manifest: 1
- **Total Transactions**: 51
- **Total Bytes**: ~100 MiB + manifest overhead

## Authorization Expiry

Every authorization has an expiry block (`expiration`):
- It is set to the grant block plus `AuthorizationPeriod` (14 days on Westend), and no later than the granting authorizer's own `valid_until`
- The authorization becomes invalid at that block; `refresh_account_authorization` / `refresh_preimage_authorization` extend it
- Unused authorization is not refunded
- Plan your uploads accordingly

## Next Steps

- [Storage Model](./storage.md) - How to store data
- [Data Retrieval](./retrieval.md) - Fetching stored data
- [Data Renewal](./renewal.md) - Extending retention
