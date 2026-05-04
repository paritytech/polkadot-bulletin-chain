// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Simulates the examples/complete-workflow.ts flow using MockBulletinClient.
 *
 * Verifies that all 10 pallet extrinsics exposed by the SDK can be
 * called with correct argument types and produce the expected results.
 */

import { blake2b } from "@noble/hashes/blake2.js"
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

    const authReceipt = await client
      .authorizeAccount(
        bobAddress,
        estimate.transactions,
        BigInt(estimate.bytes),
      )
      .send()
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
    const contentHash = blake2b(specificData.asBytes(), { dkLen: 32 })

    const preimageReceipt = await client
      .authorizePreimage(contentHash, BigInt(specificData.asBytes().length))
      .send()
    expect(preimageReceipt.blockHash).toBeDefined()

    // 4. Refresh account authorization
    const refreshAcctReceipt = await client
      .refreshAccountAuthorization(bobAddress)
      .send()
    expect(refreshAcctReceipt.blockHash).toBeDefined()

    // 5. Refresh preimage authorization
    const refreshPreimageReceipt = await client
      .refreshPreimageAuthorization(contentHash)
      .send()
    expect(refreshPreimageReceipt.blockHash).toBeDefined()

    // 6. Remove expired account authorization
    const removeAcctReceipt = await client
      .removeExpiredAccountAuthorization(bobAddress)
      .send()
    expect(removeAcctReceipt.blockHash).toBeDefined()

    // 7. Remove expired preimage authorization
    const removePreimageReceipt = await client
      .removeExpiredPreimageAuthorization(contentHash)
      .send()
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
      client.refreshAccountAuthorization(bobAddress).send(),
    ).rejects.toMatchObject({ code: "AUTHORIZATION_FAILED" })

    await expect(
      client.refreshPreimageAuthorization(new Uint8Array(32)).send(),
    ).rejects.toMatchObject({ code: "AUTHORIZATION_FAILED" })
  })

  it("should allow remove_expired calls even with simulateAuthFailure", async () => {
    const client = new MockBulletinClient({ simulateAuthFailure: true })

    // remove_expired methods don't require auth — they should succeed
    const receipt = await client
      .removeExpiredAccountAuthorization(bobAddress)
      .send()
    expect(receipt.blockHash).toBeDefined()

    const receipt2 = await client
      .removeExpiredPreimageAuthorization(new Uint8Array(32))
      .send()
    expect(receipt2.blockHash).toBeDefined()
  })

  it("should reject withCodec on chunked uploads", async () => {
    // Use a small chunkingThreshold so we don't need a huge buffer
    const client = new MockBulletinClient({ chunkingThreshold: 1024 })
    const data = new Uint8Array(2048) // exceeds 1024 threshold

    await expect(
      client.store(data).withCodec(CidCodec.DagPb).send(),
    ).rejects.toMatchObject({ code: "INVALID_CONFIG" })
  })

  it("should store chunked without manifest when withManifest(false)", async () => {
    const client = new MockBulletinClient({ chunkingThreshold: 1024 })
    const data = new Uint8Array(2048).fill(0xab)

    const result = await client
      .store(data)
      .withChunkSize(1024)
      .withManifest(false)
      .send()

    // Without manifest, cid should be undefined
    expect(result.cid).toBeUndefined()
    // Chunks should be present
    expect(result.chunks).toBeDefined()
    expect(result.chunks?.numChunks).toBe(2)
    expect(result.chunks?.chunkCids).toHaveLength(2)
  })
})
