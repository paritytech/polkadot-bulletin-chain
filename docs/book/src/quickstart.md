# Quick Start

This tutorial walks you through storing your first data on Bulletin Chain in under 5 minutes.

## Prerequisites

- Node.js 18+ or Rust toolchain
- A Polkadot-compatible wallet (Polkadot.js extension, Talisman, etc.)

## Option 1: Use the Console UI (Easiest)

The fastest way to try Bulletin Chain is through the web console:

1. **Open the Console**: Visit the deployed console or run locally (see below)
2. **Connect Wallet**: Click "Connect Wallet" and select your wallet
3. **Select Network**: Choose "Paseo" or "Westend" testnet
4. **Get Authorization**: Go to Dashboard → Click "Faucet" to get free authorization
5. **Upload Data**: Go to Upload → Select a file or enter text → Click "Upload"
6. **Done!** You'll receive a CID (Content Identifier) for your data

### Running Console Locally

```bash
# Clone the repo
git clone https://github.com/paritytech/polkadot-bulletin-chain.git
cd polkadot-bulletin-chain

# Build SDK
cd sdk/typescript && npm install && npm run build && cd ../..

# Run console
cd console-ui && npm install && npm run dev
```

Open http://localhost:5173

---

## Option 2: TypeScript SDK

### Step 1: Install

```bash
npm install @bulletin/sdk polkadot-api
```

### Step 2: Store Data

```typescript
import { calculateCid, HashAlgorithm, CidCodec } from "@bulletin/sdk";
import { createClient } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws-provider/node";
import { bulletin } from "@polkadot-api/descriptors"; // Generate with papi

async function main() {
  // 1. Connect to testnet
  const client = createClient(
    getWsProvider("wss://paseo-bulletin-rpc.polkadot.io")
  );
  const api = client.getTypedApi(bulletin);

  // 2. Prepare your data
  const data = new TextEncoder().encode("Hello, Bulletin Chain!");

  // 3. Calculate CID (what you'll use to retrieve later)
  const cid = await calculateCid(data, CidCodec.Raw, HashAlgorithm.Blake2b256);
  console.log("CID:", cid.toString());

  // 4. Submit store transaction (requires authorization - use Faucet first!)
  const signer = /* your polkadot-api signer */;

  const tx = api.tx.TransactionStorage.store({
    data: data,
    cid_config: { codec: 0x55, hashing: "Blake2b256" }
  });

  const result = await tx.signAndSubmit(signer);
  console.log("Stored in block:", result.block.number);
}

main();
```

### Step 3: Get Authorization (Required)

Before storing, you need authorization. On testnets, use the Faucet in the Console UI, or if you have sudo access:

```typescript
// Only works if you have sudo/root access
const authTx = api.tx.Sudo.sudo({
  call: api.tx.TransactionStorage.authorize_account({
    who: yourAddress,
    transactions: 10,
    bytes: 1024 * 1024  // 1 MiB
  })
});
await authTx.signAndSubmit(sudoSigner);
```

---

## Option 3: Rust SDK

### Step 1: Add Dependencies

```toml
[dependencies]
bulletin-sdk-rust = "0.1"
subxt = "0.44"
tokio = { version = "1", features = ["full"] }
```

### Step 2: Store Data

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create SDK client
    let client = BulletinClient::new();

    // 2. Prepare data
    let data = b"Hello, Bulletin Chain!".to_vec();
    let operation = client.prepare_store(data, StoreOptions::default())?;

    // 3. Calculate CID
    let cid = operation.calculate_cid()?;
    println!("CID: {}", cid.to_string());

    // 4. Connect to chain and submit (requires authorization)
    let api = subxt::OnlineClient::<subxt::PolkadotConfig>::from_url(
        "wss://paseo-bulletin-rpc.polkadot.io"
    ).await?;

    // Build and submit transaction
    let tx = bulletin::tx()
        .transaction_storage()
        .store(operation.data().to_vec(), None);

    let result = api.tx()
        .sign_and_submit_then_watch_default(&tx, &signer)
        .await?
        .wait_for_finalized_success()
        .await?;

    println!("Stored in block: {}", result.block_number());
    Ok(())
}
```

---

## What's Next?

Now that you've stored your first data:

1. **[Core Concepts](./concepts/README.md)** - Understand how Bulletin Chain works
2. **[TypeScript SDK](./typescript/README.md)** - Full TypeScript documentation
3. **[Rust SDK](./rust/README.md)** - Full Rust documentation
4. **[Authorization](./concepts/authorization.md)** - Learn about authorization management
5. **[Chunked Uploads](./typescript/chunked-uploads.md)** - Store files up to 64 MiB with automatic chunking

## Testnet Faucet

To get authorization on testnets:

1. Open the Console UI
2. Connect your wallet
3. Go to **Dashboard**
4. Click the **Faucet** button
5. Wait for the transaction to confirm

You'll receive authorization to store up to 1 MiB of data.

## Troubleshooting

| Error | Solution |
|-------|----------|
| "Unauthorized" | Get authorization via Faucet first |
| "InsufficientBalance" | Get testnet tokens from a faucet |
| "Connection refused" | Check the RPC endpoint is correct |
| "Transaction failed" | Check you have enough authorization bytes |

## Networks

| Network | RPC Endpoint | Status |
|---------|--------------|--------|
| Paseo (Testnet) | `wss://paseo-bulletin-rpc.polkadot.io` | Active |
| Westend (Testnet) | `wss://westend-bulletin-rpc.polkadot.io` | Active |
| Local Dev | `ws://localhost:10000` | - |
