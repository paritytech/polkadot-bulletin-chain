// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * Simulates the examples/complete-workflow.ts flow using MockBulletinClient.
 *
 * Verifies that the SDK's pallet extrinsics + the estimateUpload → submit
 * flow can be called with correct argument types and produce expected results.
 */

import { blake2b } from "@noble/hashes/blake2.js"
import { describe, expect, it } from "vitest"
import { blobFromBytes, blobFromItems } from "../../src/blob-source"
import { MockBulletinClient, type MockOperation } from "../../src/mock-client"
import { CidCodec, HashAlgorithm, UploadStatus } from "../../src/types"

describe("Complete workflow (MockBulletinClient)", () => {
  const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"

  it("should execute the full authorization → upload → lifecycle workflow", async () => {
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

    // 2. Upload data via estimateUpload → submit
    const message = "Hello from Bob!"
    const data = new TextEncoder().encode(message)
    const src = blobFromBytes(data)
    const { cids: uploadCids } = await client
      .submit(await client.estimateUpload(src), src)
      .send()
    expect(uploadCids[uploadCids.length - 1]).toBeDefined()

    // 3. Authorize preimage
    const specificData = new TextEncoder().encode("Preimage-authorized content")
    const contentHash = blake2b(specificData, { dkLen: 32 })

    const preimageReceipt = await client
      .authorizePreimage(contentHash, BigInt(specificData.length))
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

  it("should upload with custom per-item CID config via submit()", async () => {
    const client = new MockBulletinClient()
    const data = new TextEncoder().encode("Custom CID config test")

    // Default codec (Raw + Blake2b-256)
    const def = [{ data }]
    const { cids: defaultCids } = await client
      .submit(await client.estimateUpload(def), blobFromItems(def))
      .send()

    // DagPb codec + SHA2-256 → different CID
    const cust = [
      { data, codec: CidCodec.DagPb, hashAlgo: HashAlgorithm.Sha2_256 },
    ]
    const { cids: customCids } = await client
      .submit(await client.estimateUpload(cust), blobFromItems(cust))
      .send()

    expect(defaultCids).toHaveLength(1)
    expect(customCids).toHaveLength(1)
    expect(customCids[0]!.toString()).not.toBe(defaultCids[0]!.toString())
  })

  it("should reject uploads with empty data", async () => {
    const client = new MockBulletinClient()
    await expect(
      client.estimateUpload(blobFromBytes(new Uint8Array(0))),
    ).rejects.toMatchObject({ code: "EMPTY_DATA" })
    // An empty item list is now a no-op estimate (nothing to submit), not an error.
    const empty = await client.estimateUpload([])
    expect(empty.total).toBe(0)
  })

  it("should emit ItemStarted + ItemFinalized events through the mock", async () => {
    const client = new MockBulletinClient()
    const items = [
      { data: new TextEncoder().encode("a") },
      { data: new TextEncoder().encode("b") },
      { data: new TextEncoder().encode("c") },
    ]
    const events: Array<{ type: UploadStatus; index: number }> = []
    const { cids } = await client
      .submit(await client.estimateUpload(items), blobFromItems(items))
      .withCallback((ev) => events.push({ type: ev.type, index: ev.index }))
      .send()
    expect(cids).toHaveLength(3)
    // 3 started + 3 finalized = 6 events
    expect(
      events.filter((e) => e.type === UploadStatus.ItemStarted),
    ).toHaveLength(3)
    expect(
      events.filter((e) => e.type === UploadStatus.ItemFinalized),
    ).toHaveLength(3)
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

    const receipt = await client
      .removeExpiredAccountAuthorization(bobAddress)
      .send()
    expect(receipt.blockHash).toBeDefined()

    const receipt2 = await client
      .removeExpiredPreimageAuthorization(new Uint8Array(32))
      .send()
    expect(receipt2.blockHash).toBeDefined()
  })
})
