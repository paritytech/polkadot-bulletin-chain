// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Complete workflow example - All Bulletin Chain operations
 *
 * This example demonstrates:
 * - Account and preimage authorization
 * - Storing data with proper authorization
 * - Refreshing authorizations
 * - Renewing stored data
 * - Removing expired authorizations
 *
 * Usage:
 *   npm install
 *   npm run build
 *   node examples/complete-workflow.js
 */

import { blake2b256 } from "@noble/hashes/blake2b"
import { sr25519CreateDerive } from "@polkadot-labs/hdkd"
import { DEV_PHRASE } from "@polkadot-labs/hdkd-helpers"
import { Binary, createClient } from "polkadot-api"
import { getPolkadotSigner } from "polkadot-api/signer"
import { getWsProvider } from "polkadot-api/ws-provider/node"
import { AsyncBulletinClient } from "../dist/index.js"

async function main() {
  console.log("🚀 Bulletin SDK - Complete Workflow Example\n")

  // 1. Setup connection
  console.log("📡 Connecting to Bulletin Chain...")
  const wsProvider = getWsProvider("ws://localhost:9944")
  const papiClient = createClient(wsProvider)
  const api = papiClient.getTypedApi(/* your chain descriptors */)

  // Using Alice (sudo) for authorization operations
  const keyring = sr25519CreateDerive(DEV_PHRASE)
  const aliceSigner = getPolkadotSigner(keyring.derive("//Alice"), "Alice", 42)
  const bobSigner = getPolkadotSigner(keyring.derive("//Bob"), "Bob", 42)

  console.log("✅ Connected\n")

  // 2. Account Authorization Workflow
  console.log("═══ Account Authorization Workflow ═══\n")

  // Create client for Alice (sudo account)
  const aliceClient = new AsyncBulletinClient(
    api,
    aliceSigner,
    papiClient.submit,
  )

  // Authorize Bob's account
  const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"
  console.log("👤 Authorizing Bob:", bobAddress)

  // Calculate authorization needed for 10 MB
  const dataSize = 10 * 1024 * 1024
  const estimate = aliceClient.estimateAuthorization(dataSize)
  console.log("📊 Authorization estimate:")
  console.log("   Transactions:", estimate.transactions)
  console.log("   Bytes:", estimate.bytes, "\n")

  console.log("⏳ Authorizing account...")
  const authReceipt = await aliceClient.authorizeAccount(
    bobAddress,
    estimate.transactions,
    BigInt(estimate.bytes),
  )
  console.log("✅ Account authorized!")
  console.log("   Block:", authReceipt.blockHash)
  console.log("   Tx:", authReceipt.txHash, "\n")

  // 3. Store Data as Bob
  console.log("═══ Store Data Workflow ═══\n")

  // Create client for Bob
  const bobClient = new AsyncBulletinClient(api, bobSigner, papiClient.submit)

  const message =
    "Hello from Bob! This data is stored with proper authorization."
  const data = Binary.fromText(message)
  console.log("📝 Data:", message)
  console.log("   Size:", data.asBytes().length, "bytes\n")

  console.log("⏳ Storing data...")
  const storeResult = await bobClient.store(data).send()
  console.log("✅ Data stored!")
  console.log("   CID:", storeResult.cid.toString())
  console.log("   Block:", storeResult.blockNumber)
  console.log("   Extrinsic Index:", storeResult.extrinsicIndex, "\n")

  // 4. Preimage Authorization Workflow
  console.log("═══ Preimage Authorization Workflow ═══\n")
  console.log(
    "💡 Preimage authorization allows ANYONE to submit specific preauthorized content",
  )
  console.log(
    "   without account authorization. Storage can be submitted as unsigned transaction.\n",
  )

  const specificMessage = "This specific content is authorized by hash"
  const specificData = Binary.fromText(specificMessage)
  const contentHash = blake2b256(specificData.asBytes())

  console.log("📝 Content to authorize:")
  console.log("   Data:", specificMessage)
  console.log(
    "   Hash:",
    `${Buffer.from(contentHash).toString("hex").substring(0, 16)}...`,
    "\n",
  )

  console.log("⏳ Authorizing preimage...")
  const preimageReceipt = await aliceClient.authorizePreimage(
    contentHash,
    BigInt(specificData.asBytes().length),
  )
  console.log("✅ Preimage authorized!")
  console.log("   Block:", preimageReceipt.blockHash, "\n")

  // Anyone can now store this specific content using unsigned transaction (no fees!)
  console.log("⏳ Storing authorized preimage as unsigned transaction...")
  console.log("   💡 Unsigned = no fees, anyone can submit")
  const preimageResult = await bobClient.store(specificData).sendUnsigned()
  console.log("✅ Preimage stored (unsigned tx)!")
  console.log("   CID:", preimageResult.cid.toString())
  console.log("   No transaction fees paid! ✨\n")

  // 5. Refresh Authorization Workflow
  console.log("═══ Refresh Authorization Workflow ═══\n")

  console.log("🔄 Refreshing Bob's account authorization...")
  const refreshReceipt =
    await aliceClient.refreshAccountAuthorization(bobAddress)
  console.log("✅ Authorization refreshed!")
  console.log("   Block:", refreshReceipt.blockHash, "\n")

  console.log("🔄 Refreshing preimage authorization...")
  const refreshPreimageReceipt =
    await aliceClient.refreshPreimageAuthorization(contentHash)
  console.log("✅ Preimage authorization refreshed!")
  console.log("   Block:", refreshPreimageReceipt.blockHash, "\n")

  // 6. Renew Data Workflow
  console.log("═══ Renew Data Workflow ═══\n")

  if (
    storeResult.blockNumber !== undefined &&
    storeResult.extrinsicIndex !== undefined
  ) {
    console.log("🔄 Renewing stored data...")
    console.log("   Original block:", storeResult.blockNumber)
    console.log("   Extrinsic index:", storeResult.extrinsicIndex)

    try {
      // Use the extrinsic index from the Stored event (not hardcoded!)
      const renewReceipt = await bobClient.renew(
        storeResult.blockNumber,
        storeResult.extrinsicIndex,
      )
      console.log("✅ Data renewed!")
      console.log("   Block:", renewReceipt.blockHash, "\n")
    } catch (error) {
      console.log("ℹ️  Could not renew (may not be renewable yet)")
      console.log("   Error:", (error as Error).message, "\n")
    }
  } else {
    console.log("ℹ️  Skipping renew - missing block number or extrinsic index")
    console.log("   Block number:", storeResult.blockNumber)
    console.log("   Extrinsic index:", storeResult.extrinsicIndex)
    console.log(
      "\n   💡 The extrinsic index comes from the Stored event emitted by the pallet.",
    )
    console.log(
      "      It identifies the transaction's position within the block.\n",
    )
  }

  // 7. Remove Expired Authorization Workflow
  console.log("═══ Remove Expired Authorization Workflow ═══\n")
  console.log(
    "💡 Note: These will only work if authorizations have actually expired\n",
  )

  // Try to remove expired account authorization
  try {
    console.log("⏳ Checking for expired account authorizations...")
    const removeReceipt =
      await aliceClient.removeExpiredAccountAuthorization(bobAddress)
    console.log("✅ Expired authorization removed!")
    console.log("   Block:", removeReceipt.blockHash)
  } catch (_error) {
    console.log("ℹ️  No expired authorization found (this is normal)")
  }

  console.log()

  // Try to remove expired preimage authorization
  try {
    console.log("⏳ Checking for expired preimage authorizations...")
    const removeReceipt =
      await aliceClient.removeExpiredPreimageAuthorization(contentHash)
    console.log("✅ Expired preimage authorization removed!")
    console.log("   Block:", removeReceipt.blockHash)
  } catch (_error) {
    console.log("ℹ️  No expired preimage authorization found (this is normal)")
  }

  // 8. Summary
  console.log("\n═══ Workflow Complete ═══\n")
  console.log("✅ Demonstrated operations:")
  console.log("   • Account authorization (Alice authorizes Bob)")
  console.log("   • Signed storage (Bob stores with account auth, pays fees)")
  console.log("   • Preimage authorization (content-addressed)")
  console.log(
    "   • Unsigned storage (anyone stores preauthorized content, no fees!)",
  )
  console.log("   • Refresh authorizations (extends expiry)")
  console.log("   • Renew stored data (extends retention)")
  console.log("   • Remove expired authorizations (cleanup)")

  console.log("\n💡 Best Practices:")
  console.log("   • Authorize before storing to ensure capacity")
  console.log("   • Use account auth for dynamic content (signed tx)")
  console.log(
    "   • Use preimage auth when content is known ahead (unsigned tx, no fees!)",
  )
  console.log("   • Refresh authorizations before they expire")
  console.log("   • Renew important data before retention period ends")
  console.log("   • Clean up expired authorizations to free storage")

  console.log("\n💡 Signed vs Unsigned Transactions:")
  console.log(
    "   • Signed (.send()): Uses account authorization, requires fees",
  )
  console.log(
    "   • Unsigned (.sendUnsigned()): Uses preimage authorization, no fees!",
  )
  console.log("   • Anyone can submit unsigned tx for preauthorized content")

  console.log("\n🎉 Complete workflow example finished!")

  // Cleanup
  await papiClient.destroy()
}

main().catch(console.error)
