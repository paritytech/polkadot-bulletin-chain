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
npm install @parity/bulletin-sdk polkadot-api
```

### Step 2: Store Data

```typescript
import { AsyncBulletinClient } from "@parity/bulletin-sdk";
import { createClient, Binary } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws";
import { bulletin } from "@polkadot-api/descriptors"; // Generate with papi

async function main() {
  // 1. Connect to testnet
  const papiClient = createClient(
    getWsProvider("wss://paseo-bulletin-rpc.polkadot.io")
  );
  const api = papiClient.getTypedApi(bulletin);

  // 2. Create SDK client with PAPI client, signer, and submit function
  const client = new AsyncBulletinClient(api, signer, papiClient.submit);

  // 3. Store data (requires authorization - use Faucet first!)
  const data = Binary.fromText("Hello, Bulletin Chain!");
  const result = await client.store(data).send();

  console.log("CID:", result.cid.toString());
  console.log("Stored in block:", result.blockNumber);
}

main();
```

### Step 3: Get Authorization (Required)

Before storing, you need authorization. On testnets, use the Faucet in the Console UI, or if you have sudo access:

```typescript
// Only works if you have sudo/root access
await client.authorizeAccount(yourAddress, 10, BigInt(1024 * 1024)).withSudo().send();
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
use subxt_signer::sr25519::Keypair;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Connect to Bulletin Chain
    let client = TransactionClient::new("wss://paseo-bulletin-rpc.polkadot.io").await?;

    // 2. Create signer (replace with your key)
    let uri = subxt_signer::SecretUri::from_str("//Alice")?;
    let signer = Keypair::from_uri(&uri)?;

    // 3. Store data (requires authorization - use Faucet first!)
    let data = b"Hello, Bulletin Chain!".to_vec();
    let receipt = client.store(data, &signer).await?;

    println!("Stored {} bytes in block: {}", receipt.data_size, receipt.block_hash);
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
