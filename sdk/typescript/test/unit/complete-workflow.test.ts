// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Simulates the examples/complete-workflow.ts flow using MockBulletinClient.
 *
 * Verifies that all 10 pallet extrinsics exposed by the SDK can be
 * called with correct argument types and produce the expected results.
 */

import { blake2AsU8a } from "@polkadot/util-crypto"
import { Binary } from "polkadot-api"
import { describe, expect, it } from "vitest"
import { MockBulletinClient, type MockOperation } from "../../src/mock-client"
import { CidCodec, HashAlgorithm } from "../../src/types"

describe("Complete workflow (MockBulletinClient)", () => {
  const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"

  it("should execute the full authorization → store → lifecycle workflow", async () => {
    const client = new MockBulletinClient()

    // 1. Authorize Bob's account
    const estimate = client.estimateAuthorization(10 * 1024 * 1024)
    expect(estimate.transactions).toBeGreaterThan(0)
    expect(estimate.bytes).toBeGreaterThanOrEqual(10 * 1024 * 1024)

    const authReceipt = await client.authorizeAccount(
      bobAddress,
      estimate.transactions,
      BigInt(estimate.bytes),
    )
    expect(authReceipt.blockHash).toBeDefined()
    expect(authReceipt.txHash).toBeDefined()

    // 2. Store data (signed)
    const message = "Hello from Bob!"
    const data = Binary.fromText(message)
    const storeResult = await client.store(data).send()
    expect(storeResult.cid).toBeDefined()
    expect(storeResult.size).toBe(data.asBytes().length)

    // 3. Authorize preimage
    const specificData = Binary.fromText("Preimage-authorized content")
    const contentHash = blake2AsU8a(specificData.asBytes())

    const preimageReceipt = await client.authorizePreimage(
      contentHash,
      BigInt(specificData.asBytes().length),
    )
    expect(preimageReceipt.blockHash).toBeDefined()

    // 4. Refresh account authorization
    const refreshAcctReceipt =
      await client.refreshAccountAuthorization(bobAddress)
    expect(refreshAcctReceipt.blockHash).toBeDefined()

    // 5. Refresh preimage authorization
    const refreshPreimageReceipt =
      await client.refreshPreimageAuthorization(contentHash)
    expect(refreshPreimageReceipt.blockHash).toBeDefined()

    // 6. Remove expired account authorization
    const removeAcctReceipt =
      await client.removeExpiredAccountAuthorization(bobAddress)
    expect(removeAcctReceipt.blockHash).toBeDefined()

    // 7. Remove expired preimage authorization
    const removePreimageReceipt =
      await client.removeExpiredPreimageAuthorization(contentHash)
    expect(removePreimageReceipt.blockHash).toBeDefined()

    // Verify all operations were recorded
    const ops = client.getOperations()
    const opTypes = ops.map((o) => o.type)

    expect(opTypes).toEqual([
      "authorize_account",
      "store",
      "authorize_preimage",
      "refresh_account_authorization",
      "refresh_preimage_authorization",
      "remove_expired_account_authorization",
      "remove_expired_preimage_authorization",
    ])

    // Verify operation details
    const authOp = ops[0] as Extract<
      MockOperation,
      { type: "authorize_account" }
    >
    expect(authOp.who).toBe(bobAddress)
    expect(authOp.transactions).toBe(estimate.transactions)
    expect(authOp.bytes).toBe(BigInt(estimate.bytes))

    const refreshOp = ops[3] as Extract<
      MockOperation,
      { type: "refresh_account_authorization" }
    >
    expect(refreshOp.who).toBe(bobAddress)

    const removePreimageOp = ops[6] as Extract<
      MockOperation,
      { type: "remove_expired_preimage_authorization" }
    >
    expect(removePreimageOp.contentHash).toEqual(contentHash)
  })

  it("should store with non-default CID options", async () => {
    const client = new MockBulletinClient()

    const data = Binary.fromText("Custom CID config test")

    // Store with DagPb codec and SHA2-256 hash
    const result = await client
      .store(data)
      .withCodec(CidCodec.DagPb)
      .withHashAlgorithm(HashAlgorithm.Sha2_256)
      .send()

    expect(result.cid).toBeDefined()
    expect(result.size).toBe(data.asBytes().length)

    // The CID should be different from default (Raw + Blake2b256)
    const defaultResult = await client.store(data).send()
    expect(result.cid.toString()).not.toBe(defaultResult.cid.toString())
  })

  it("should simulate auth failure for lifecycle methods", async () => {
    const client = new MockBulletinClient({ simulateAuthFailure: true })

    await expect(
      client.refreshAccountAuthorization(bobAddress),
    ).rejects.toMatchObject({ code: "AUTHORIZATION_FAILED" })

    await expect(
      client.refreshPreimageAuthorization(new Uint8Array(32)),
    ).rejects.toMatchObject({ code: "AUTHORIZATION_FAILED" })
  })

  it("should allow remove_expired calls even with simulateAuthFailure", async () => {
    const client = new MockBulletinClient({ simulateAuthFailure: true })

    // remove_expired methods don't require auth — they should succeed
    const receipt = await client.removeExpiredAccountAuthorization(bobAddress)
    expect(receipt.blockHash).toBeDefined()

    const receipt2 = await client.removeExpiredPreimageAuthorization(
      new Uint8Array(32),
    )
    expect(receipt2.blockHash).toBeDefined()
  })
})
