# Quick Start

This tutorial walks you through storing your first data on Bulletin Chain in under 5 minutes.

## Prerequisites

- Node.js 22+ or Rust toolchain
- A Polkadot-compatible wallet (Polkadot.js extension, Talisman, PolkaGate, etc.)

## Option 1: Use the Console UI (Easiest)

The fastest way to try Bulletin Chain is through the web console:

1. **Open the Console**: Visit the deployed console or run locally (see below)
2. **Connect Wallet**: Click "Connect Wallet" and select your wallet
3. **Select Network**: Choose "Paseo" or "Westend" testnet
4. **Get Authorization**: Open "Faucet" in the menu and submit the form to get free authorization
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
npm install @parity/bulletin-sdk polkadot-api multiformats
```

### Step 2: Store Data

```typescript
import { BulletinClient, blobFromBytes } from "@parity/bulletin-sdk";
import { getWsProvider } from "polkadot-api/ws";
import { bulletin } from "@polkadot-api/descriptors"; // Generate with papi

async function main() {
  // 1. Create the client — it owns its PAPI connection.
  const client = new BulletinClient({
    providers: () => [getWsProvider("wss://paseo-bulletin-next-rpc.polkadot.io")],
    uploadSigner: signer,
    descriptor: bulletin, // optional; omit to use getUnsafeApi()
  });

  // 2. Estimate (lets a UI preview cost), then submit. Requires
  //    authorization — use the Faucet first!
  const src = blobFromBytes(new TextEncoder().encode("Hello, Bulletin Chain!"));
  const { cids } = await client.submit(await client.estimateUpload(src), src).send();

  // Last CID is the retrieval id: the manifest root, or the lone chunk's CID.
  console.log("CID:", cids[cids.length - 1].toString());
}

main();
```

### Step 3: Get Authorization (Required)

Before storing, you need authorization. On testnets, use the Faucet in the Console UI. If your account is a registered authorizer (in the chain's `AllowedAuthorizers` set), grant it yourself:

```typescript
// Pass the authorizer as `authorizerSigner` when constructing the client.
// Holding only the sudo key? Add `.withSudo()` before `.send()`.
await client.authorizeAccount(yourAddress, 10, BigInt(1024 * 1024)).send();
```

---

## Option 3: Rust SDK

### Step 1: Add Dependencies

```toml
[dependencies]
bulletin-sdk-rust = { git = "https://github.com/paritytech/polkadot-bulletin-chain" }
subxt = "0.44"
subxt-signer = { version = "0.44", features = ["sr25519"] }
tokio = { version = "1", features = ["full"] }
```

### Step 2: Store Data

```rust
use bulletin_sdk_rust::prelude::*;
use subxt_signer::sr25519::Keypair;
use std::str::FromStr;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // 1. Connect to Bulletin Chain
    let client = TransactionClient::new("wss://paseo-bulletin-next-rpc.polkadot.io").await?;

    // 2. Create signer (replace with your key)
    let uri = subxt_signer::SecretUri::from_str("//Alice")?;
    let signer = Keypair::from_uri(&uri)?;

    // 3. Store data (requires authorization - use Faucet first!)
    let data = b"Hello, Bulletin Chain!".to_vec();
    let receipt = client.store(data, &signer, WaitFor::InBlock).await?;

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
5. **[Chunked Uploads](./typescript/chunked-uploads.md)** - Store large files with streamed, automatic chunking

## Testnet Faucet

To get authorization on testnets:

1. Open the Console UI
2. Open **Faucet** in the menu
3. Enter your account and submit
4. Wait for the transaction to confirm

The faucet signs with its own authorizer key, so no wallet confirmation is needed. Its form defaults to 100 transactions and 10 MB (editable).

## Troubleshooting

| Error | Solution |
|-------|----------|
| `Invalid: Payment` or `AUTHORIZATION_NOT_FOUND` (Rust `store()`, TypeScript `.ensureAuthorized()`) | No active authorization: get one via the Faucet |
| "Connection refused" | Check the RPC endpoint is correct |
| Stores take long to be included | The allowance is used up: stores beyond it still go through, at lower priority. Top up via the Faucet |

## Networks

| Network | RPC Endpoint | Status |
|---------|--------------|--------|
| Paseo (Testnet) | `wss://paseo-bulletin-next-rpc.polkadot.io` | Active |
| Westend (Testnet) | `wss://westend-bulletin-rpc.polkadot.io` | Active |
| Local Dev | `ws://localhost:10000` | - |
