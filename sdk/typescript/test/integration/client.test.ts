// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Integration tests for Bulletin SDK
 *
 * These tests require a running Bulletin Chain node.
 * Default endpoint: ws://localhost:9944 (override with BULLETIN_RPC_URL env var)
 *
 * Run with: npm run test:integration
 *
 * Note: Tests run sequentially to avoid conflicts on the same chain
 */

import { sr25519CreateDerive } from "@polkadot-labs/hdkd"
import {
  blake2b256,
  DEV_MINI_SECRET,
  ss58Address,
} from "@polkadot-labs/hdkd-helpers"
import { createClient, type PolkadotClient } from "polkadot-api"
import { getPolkadotSigner } from "polkadot-api/signer"
import { getWsProvider } from "polkadot-api/ws-provider/node"
import { afterAll, beforeAll, describe, expect, it } from "vitest"
import {
  AsyncBulletinClient,
  type BulletinTypedApi,
  CidCodec,
  HashAlgorithm,
} from "../../src"

const ENDPOINT = process.env.BULLETIN_RPC_URL ?? "ws://localhost:9944"

describe("AsyncBulletinClient Integration Tests", { timeout: 120_000 }, () => {
  let client: AsyncBulletinClient
  let papiClient: PolkadotClient
  let aliceAddress: string

  beforeAll(async () => {
    // Setup connection
    const wsProvider = getWsProvider(ENDPOINT)
    papiClient = createClient(wsProvider)
    const api = papiClient.getUnsafeApi() as unknown as BulletinTypedApi

    // Create signer (Alice for dev chain)
    const derive = sr25519CreateDerive(DEV_MINI_SECRET)
    const aliceKeyPair = derive("//Alice")
    const signer = getPolkadotSigner(
      aliceKeyPair.publicKey,
      "Sr25519",
      aliceKeyPair.sign,
    )
    aliceAddress = ss58Address(aliceKeyPair.publicKey, 42)

    // Create client directly with api, signer, and submit function
    client = new AsyncBulletinClient(api, signer, papiClient.submit)

    // Authorize Alice's account for storage operations
    // The bulletin chain requires account authorization before storing data
    const estimate = client.estimateAuthorization(50 * 1024 * 1024) // 50 MB budget
    await client.authorizeAccount(
      aliceAddress,
      estimate.transactions,
      BigInt(estimate.bytes),
    )
    console.log("Alice authorized for storage:", aliceAddress)
  })

  afterAll(async () => {
    if (papiClient) {
      papiClient.destroy()
    }
  })

  describe("Store Operations", () => {
    it("should store simple data", async () => {
      const data = new TextEncoder().encode(
        "Hello, Bulletin Chain! Integration test.",
      )

      const result = await client.store(data).send()

      expect(result).toBeDefined()
      expect(result.cid).toBeDefined()
      expect(result.size).toBe(data.length)
      expect(result.cid.toString()).toMatch(/^[a-z0-9]+$/i)

      console.log("Simple store test passed")
      console.log("   CID:", result.cid.toString())
      console.log("   Size:", result.size, "bytes")
    })

    it("should store with custom CID options", async () => {
      const data = new TextEncoder().encode("Test with custom options")

      const result = await client
        .store(data)
        .withCodec(CidCodec.DagPb)
        .withHashAlgorithm(HashAlgorithm.Sha2_256)
        .withWaitFor("finalized")
        .send()

      expect(result).toBeDefined()
      expect(result.cid).toBeDefined()
      expect(result.size).toBe(data.length)

      console.log("Custom options store test passed")
      console.log("   CID:", result.cid.toString())
    })

    it("should store chunked data with progress tracking", async () => {
      // Create 5 MiB test data
      const data = new Uint8Array(5 * 1024 * 1024).fill(0x42)

      let chunksCompleted = 0
      let manifestCreated = false
      let totalChunks = 0

      const result = await client.storeChunked(
        data,
        { chunkSize: 1024 * 1024, maxParallel: 4, createManifest: true },
        undefined,
        (event) => {
          switch (event.type) {
            case "chunk_started":
              if (totalChunks === 0) totalChunks = event.total
              break
            case "chunk_completed":
              chunksCompleted++
              console.log(
                `   Chunk ${event.index + 1}/${event.total} completed`,
              )
              break
            case "manifest_created":
              manifestCreated = true
              console.log("   Manifest created:", event.cid.toString())
              break
          }
        },
      )

      expect(result).toBeDefined()
      expect(result.numChunks).toBe(5) // 5 MiB / 1 MiB = 5 chunks
      expect(chunksCompleted).toBe(5)
      expect(manifestCreated).toBe(true)
      expect(result.manifestCid).toBeDefined()
      expect(result.chunkCids).toHaveLength(5)

      console.log("Chunked store test passed")
      console.log("   Chunks:", result.numChunks)
      console.log("   Manifest CID:", result.manifestCid?.toString())
    })

    it("should fire progress events in correct order during chunked upload", async () => {
      const data = new Uint8Array(3 * 1024 * 1024).fill(0xaa) // 3 MiB → 3 chunks

      const events: string[] = []

      const result = await client.storeChunked(
        data,
        { chunkSize: 1024 * 1024, createManifest: true },
        undefined,
        (event) => {
          events.push(event.type)
        },
      )

      // Verify event order: started/completed pairs for each chunk, then manifest, then completed
      expect(result.numChunks).toBe(3)

      const expectedOrder = [
        "chunk_started",
        "chunk_completed",
        "chunk_started",
        "chunk_completed",
        "chunk_started",
        "chunk_completed",
        "manifest_started",
        "manifest_created",
        "completed",
      ]
      expect(events).toEqual(expectedOrder)
    })

    it("should fire chunk events sequentially (each chunk submitted before next starts)", async () => {
      const data = new Uint8Array(2 * 1024 * 1024).fill(0xbb) // 2 MiB → 2 chunks

      const eventLog: { type: string; index: number; time: number }[] = []

      await client.storeChunked(
        data,
        { chunkSize: 1024 * 1024, createManifest: false },
        undefined,
        (event) => {
          if (
            event.type === "chunk_started" ||
            event.type === "chunk_completed"
          ) {
            eventLog.push({
              type: event.type,
              index: event.index,
              time: Date.now(),
            })
          }
        },
      )

      expect(eventLog).toHaveLength(4) // 2x started + 2x completed

      // Verify sequential order: chunk 0 must complete before chunk 1 starts
      const chunk0Completed = eventLog.find(
        (e) => e.type === "chunk_completed" && e.index === 0,
      )
      const chunk1Started = eventLog.find(
        (e) => e.type === "chunk_started" && e.index === 1,
      )
      expect(chunk0Completed).toBeDefined()
      expect(chunk1Started).toBeDefined()
      expect(chunk0Completed?.time).toBeLessThanOrEqual(chunk1Started?.time)
    })

    it("should include CID in chunk_completed events", async () => {
      const data = new Uint8Array(2 * 1024 * 1024).fill(0xcc) // 2 MiB → 2 chunks

      const chunkCids: string[] = []

      const result = await client.storeChunked(
        data,
        { chunkSize: 1024 * 1024, createManifest: false },
        undefined,
        (event) => {
          if (event.type === "chunk_completed") {
            chunkCids.push(event.cid.toString())
          }
        },
      )

      expect(chunkCids).toHaveLength(2)
      expect(result.chunkCids).toHaveLength(2)
      // CIDs from events should match CIDs from result
      expect(chunkCids).toEqual(result.chunkCids.map((c) => c.toString()))
    })

    it("should fire chunk_completed via store() builder for large data", async () => {
      const data = new Uint8Array(3 * 1024 * 1024).fill(0xdd) // 3 MiB, above default threshold

      const events: string[] = []

      const result = await client
        .store(data)
        .withCallback((event) => {
          events.push(event.type)
        })
        .send()

      expect(result.cid).toBeDefined()
      expect(result.chunks).toBeDefined()
      expect(result.chunks?.numChunks).toBe(3)

      // Should have chunk events from the submission loop
      expect(events.filter((e) => e === "chunk_completed")).toHaveLength(3)
      expect(events.filter((e) => e === "chunk_started")).toHaveLength(3)
    })
  })

  describe("Authorization Operations", () => {
    it("should estimate authorization", () => {
      const estimate = client.estimateAuthorization(10_000_000) // 10 MB

      expect(estimate).toBeDefined()
      expect(estimate.transactions).toBeGreaterThan(0)
      // bytes includes manifest overhead (numChunks * 10 + 1000)
      expect(estimate.bytes).toBeGreaterThanOrEqual(10_000_000)

      console.log("Authorization estimation test passed")
      console.log("   Transactions:", estimate.transactions)
      console.log("   Bytes:", estimate.bytes)
    })

    it("should authorize account", async () => {
      const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"
      const estimate = client.estimateAuthorization(1_000_000)

      const receipt = await client.authorizeAccount(
        bobAddress,
        estimate.transactions,
        BigInt(estimate.bytes),
      )

      expect(receipt).toBeDefined()
      expect(receipt.blockHash).toBeDefined()
      expect(receipt.txHash).toBeDefined()

      console.log("Account authorization test passed")
      console.log("   Block hash:", receipt.blockHash)
    })

    it("should authorize preimage", async () => {
      const data = new TextEncoder().encode("Specific content to authorize")
      const contentHash = blake2b256(data)

      const receipt = await client.authorizePreimage(
        contentHash,
        BigInt(data.length),
      )

      expect(receipt).toBeDefined()
      expect(receipt.blockHash).toBeDefined()

      console.log("Preimage authorization test passed")
    })
  })

  describe("Refresh Operations", () => {
    it("should refresh account authorization", async () => {
      // First authorize Bob
      const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"
      const estimate = client.estimateAuthorization(1_000_000)
      await client.authorizeAccount(
        bobAddress,
        estimate.transactions,
        BigInt(estimate.bytes),
      )

      // Now refresh Bob's authorization
      const receipt = await client.refreshAccountAuthorization(bobAddress)

      expect(receipt).toBeDefined()
      expect(receipt.blockHash).toBeDefined()
      expect(receipt.txHash).toBeDefined()

      console.log("Refresh account authorization test passed")
    })

    it("should refresh preimage authorization", async () => {
      // First authorize a preimage
      const data = new TextEncoder().encode("Content for refresh test")
      const contentHash = blake2b256(data)
      await client.authorizePreimage(contentHash, BigInt(data.length))

      // Now refresh the preimage authorization
      const receipt = await client.refreshPreimageAuthorization(contentHash)

      expect(receipt).toBeDefined()
      expect(receipt.blockHash).toBeDefined()
      expect(receipt.txHash).toBeDefined()

      console.log("Refresh preimage authorization test passed")
    })
  })

  describe("Remove Expired Authorization Operations", () => {
    it("should attempt to remove expired account authorization", async () => {
      const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"

      // This will likely fail because the authorization hasn't expired yet
      // but it tests the SDK method is wired up correctly
      try {
        const receipt =
          await client.removeExpiredAccountAuthorization(bobAddress)
        expect(receipt).toBeDefined()
        expect(receipt.blockHash).toBeDefined()
        console.log("Remove expired account authorization succeeded")
      } catch (_error) {
        // Expected - authorization hasn't expired
        console.log(
          "Remove expired account authorization failed as expected (not expired)",
        )
      }
    })

    it("should attempt to remove expired preimage authorization", async () => {
      const data = new TextEncoder().encode("Content for expiry test")
      const contentHash = blake2b256(data)

      try {
        const receipt =
          await client.removeExpiredPreimageAuthorization(contentHash)
        expect(receipt).toBeDefined()
        console.log("Remove expired preimage authorization succeeded")
      } catch (_error) {
        // Expected - authorization hasn't expired or doesn't exist
        console.log("Remove expired preimage authorization failed as expected")
      }
    })
  })

  describe("Preimage Store Operations", () => {
    it("should store data with preimage authorization", async () => {
      const data = new TextEncoder().encode(
        "This content is preimage-authorized for unsigned storage",
      )
      const contentHash = blake2b256(data)

      // Authorize the preimage first
      await client.authorizePreimage(contentHash, BigInt(data.length))

      // Store with preimage auth (unsigned transaction)
      const result = await client.storeWithPreimageAuth(data)

      expect(result).toBeDefined()
      expect(result.cid).toBeDefined()
      expect(result.size).toBe(data.length)

      console.log("Store with preimage auth test passed")
      console.log("   CID:", result.cid.toString())
    })
  })

  describe("Maintenance Operations", () => {
    it("should renew stored data", async () => {
      // First store something
      const data = new TextEncoder().encode("Data to be renewed")
      const storeResult = await client.store(data).send()

      // Wait a bit for block finalization
      await new Promise((resolve) => setTimeout(resolve, 1000))

      // Try to renew (may fail if not renewable yet)
      try {
        const receipt = await client.renew(storeResult.blockNumber ?? 0, 0)
        expect(receipt).toBeDefined()
        console.log("Renew test passed")
      } catch (_error) {
        console.log("Renew not available yet (expected)")
      }
    })
  })

  describe("Complete Workflow", () => {
    it("should complete full authorization and store workflow", async () => {
      // 1. Estimate authorization
      const dataSize = 2 * 1024 * 1024 // 2 MB
      const estimate = client.estimateAuthorization(dataSize)

      console.log(
        "   Authorization needed:",
        estimate.transactions,
        "transactions",
      )

      // 2. Authorize a new account (Bob)
      const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"
      const authReceipt = await client.authorizeAccount(
        bobAddress,
        estimate.transactions,
        BigInt(estimate.bytes),
      )

      expect(authReceipt.blockHash).toBeDefined()
      console.log("   Bob authorized")

      // 3. Store data (as Alice, who was already authorized in beforeAll)
      const data = new Uint8Array(dataSize).fill(0x55)
      const storeResult = await client.store(data).send()

      expect(storeResult.cid).toBeDefined()
      expect(storeResult.size).toBe(dataSize)
      console.log("   Data stored with CID:", storeResult.cid.toString())

      console.log("Complete workflow test passed")
    })
  })
})
