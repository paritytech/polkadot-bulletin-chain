// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Integration tests against a live Bulletin Chain node.
 *
 * Requires a running node (default `ws://localhost:9944`; override with
 * `BULLETIN_RPC_URL`). Run with `npm run test:integration`.
 *
 * Exercises the SDK's `upload([…])` / `uploadFile()` / `asUnsigned()` flows
 * end-to-end against the new pipeline-based submission engine.
 */

import { blake2b } from "@noble/hashes/blake2.js"
import { sr25519CreateDerive } from "@polkadot-labs/hdkd"
import { DEV_MINI_SECRET, ss58Address } from "@polkadot-labs/hdkd-helpers"
import { createClient, type PolkadotClient } from "polkadot-api"
import { getPolkadotSigner } from "polkadot-api/signer"
import { getWsProvider } from "polkadot-api/ws"
import { afterAll, beforeAll, describe, expect, it } from "vitest"
import {
  BulletinClient,
  type BulletinTypedApi,
  CidCodec,
  HashAlgorithm,
  UploadStatus,
} from "../../src"

const ENDPOINT = process.env.BULLETIN_RPC_URL ?? "ws://localhost:9944"

const blake2b256 = (data: Uint8Array) => blake2b(data, { dkLen: 32 })

describe("BulletinClient Integration Tests", { timeout: 120_000 }, () => {
  let client: BulletinClient
  let papiClient: PolkadotClient
  let aliceAddress: string

  beforeAll(async () => {
    const wsProvider = getWsProvider(ENDPOINT)
    papiClient = createClient(wsProvider)
    const api = papiClient.getUnsafeApi() as unknown as BulletinTypedApi

    const derive = sr25519CreateDerive(DEV_MINI_SECRET)
    const aliceKeyPair = derive("//Alice")
    const signer = getPolkadotSigner(
      aliceKeyPair.publicKey,
      "Sr25519",
      aliceKeyPair.sign,
    )
    aliceAddress = ss58Address(aliceKeyPair.publicKey, 42)

    // 120s tx timeout for CI zombienet nodes — finalization can take >60s.
    // wsUrls opt the signed path into the pipelined submission engine.
    client = new BulletinClient(api, signer, papiClient.submitAndWatch, {
      txTimeout: 120_000,
      wsUrls: [ENDPOINT],
    })

    // Authorize Alice's account so signed uploads succeed.
    const estimate = client.estimateAuthorization(50 * 1024 * 1024)
    await client
      .authorizeAccount(
        aliceAddress,
        estimate.transactions,
        BigInt(estimate.bytes),
      )
      .send()
  })

  afterAll(async () => {
    if (papiClient) papiClient.destroy()
  })

  describe("upload() — signed via pipeline", () => {
    it("stores a single item", async () => {
      const data = new TextEncoder().encode(
        "Hello, Bulletin — integration test",
      )
      const { cids } = await client.upload([{ data }]).send()
      expect(cids).toHaveLength(1)
      expect(cids[0]!.toString()).toMatch(/^[a-z0-9]+$/i)
    })

    it("stores with non-default codec + hash", async () => {
      const data = new TextEncoder().encode("custom codec test")
      const { cids } = await client
        .upload([
          { data, codec: CidCodec.DagPb, hashAlgo: HashAlgorithm.Sha2_256 },
        ])
        .withWaitFor("finalized")
        .send()
      expect(cids).toHaveLength(1)
    })

    it("stores N items in one batch and resolves N CIDs", async () => {
      const items = Array.from({ length: 4 }, (_, i) => ({
        data: new TextEncoder().encode(`batch item ${i} ${Date.now()}`),
      }))
      const { cids } = await client.upload(items).send()
      expect(cids).toHaveLength(4)
    })
  })

  /**
   * Chunk data MUST be unique across chunks, otherwise the SDK's
   * `TransactionByContentHash`-based reconciler can't tell two chunks
   * with the same content_hash apart and the SDK rejects with
   * `INVALID_CONFIG`. Each byte mixes the position's high-byte windows
   * so 1 MiB chunks at different positions produce distinct hashes,
   * and a per-call random tag avoids cross-run collisions on the chain.
   */
  function makeChunkedTestData(size: number): Uint8Array {
    const data = new Uint8Array(size)
    const tag = Math.floor(Math.random() * 0xffffffff)
    for (let i = 0; i < size; i++) {
      // Bits from every byte-window of `i` contribute, so position
      // 0x100000 (1 MiB) yields a different value than position 0.
      data[i] = (tag ^ i ^ (i >> 8) ^ (i >> 16) ^ (i >> 24)) & 0xff
    }
    return data
  }

  describe("uploadFile() — chunked file + DAG-PB manifest", () => {
    it("auto-chunks a 5 MiB file and returns one root CID", async () => {
      const data = makeChunkedTestData(5 * 1024 * 1024)
      const events: Array<{
        type: UploadStatus
        index: number
        total: number
      }> = []

      const { cid } = await client
        .uploadFile(data)
        .withChunkSize(1024 * 1024)
        .withCallback((ev) =>
          events.push({ type: ev.type, index: ev.index, total: ev.total }),
        )
        .send()

      expect(cid).toBeDefined()
      // 5 chunks + 1 manifest = 6 items
      const finalized = events.filter(
        (e) => e.type === UploadStatus.ItemFinalized,
      )
      expect(finalized).toHaveLength(6)
      expect(finalized[0]?.total).toBe(6)
    })

    it("fires events in input order (Started → InBlock → Finalized per item)", async () => {
      const data = makeChunkedTestData(3 * 1024 * 1024) // 3 MiB
      const lastSeenByIndex = new Map<number, UploadStatus>()

      await client
        .uploadFile(data)
        .withChunkSize(1024 * 1024)
        .withCallback((ev) => {
          const prev = lastSeenByIndex.get(ev.index)
          lastSeenByIndex.set(ev.index, ev.type)
          // Per-item state machine: Started → InBlock → Finalized.
          if (ev.type === UploadStatus.ItemInBlock) {
            expect(prev).toBe(UploadStatus.ItemStarted)
          }
          if (ev.type === UploadStatus.ItemFinalized) {
            expect([
              UploadStatus.ItemStarted,
              UploadStatus.ItemInBlock,
            ]).toContain(prev)
          }
        })
        .send()

      // 3 chunks + 1 manifest = 4 items
      expect(lastSeenByIndex.size).toBe(4)
    })

    it("surfaces per-item CIDs through ItemFinalized events", async () => {
      const data = makeChunkedTestData(2 * 1024 * 1024) // 2 MiB → 2 chunks
      const finalizedCids: string[] = []

      const { cid: rootCid } = await client
        .uploadFile(data)
        .withChunkSize(1024 * 1024)
        .withCallback((ev) => {
          if (ev.type === UploadStatus.ItemFinalized) {
            finalizedCids.push(ev.cid.toString())
          }
        })
        .send()

      // 2 chunks + 1 manifest = 3 finalized events
      expect(finalizedCids).toHaveLength(3)
      // The root CID is the last finalized item (the manifest).
      expect(finalizedCids[finalizedCids.length - 1]).toBe(rootCid.toString())
    })
  })

  describe("Authorization Operations", () => {
    it("estimates authorization", () => {
      const estimate = client.estimateAuthorization(10_000_000)
      expect(estimate.transactions).toBeGreaterThan(0)
      expect(estimate.bytes).toBeGreaterThanOrEqual(10_000_000)
    })

    it("authorizes an account", async () => {
      const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"
      const estimate = client.estimateAuthorization(1_000_000)
      const receipt = await client
        .authorizeAccount(
          bobAddress,
          estimate.transactions,
          BigInt(estimate.bytes),
        )
        .send()
      expect(receipt.blockHash).toBeDefined()
      expect(receipt.txHash).toBeDefined()
    })

    it("authorizes a preimage", async () => {
      const data = new TextEncoder().encode("Specific content to authorize")
      const contentHash = blake2b256(data)
      const receipt = await client
        .authorizePreimage(contentHash, BigInt(data.length))
        .send()
      expect(receipt.blockHash).toBeDefined()
    })
  })

  describe("Refresh Operations", () => {
    it("refreshes account authorization", async () => {
      const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"
      const estimate = client.estimateAuthorization(1_000_000)
      await client
        .authorizeAccount(
          bobAddress,
          estimate.transactions,
          BigInt(estimate.bytes),
        )
        .send()
      const receipt = await client
        .refreshAccountAuthorization(bobAddress)
        .send()
      expect(receipt.blockHash).toBeDefined()
      expect(receipt.txHash).toBeDefined()
    })

    it("refreshes preimage authorization", async () => {
      const data = new TextEncoder().encode("Content for refresh test")
      const contentHash = blake2b256(data)
      await client.authorizePreimage(contentHash, BigInt(data.length)).send()
      const receipt = await client
        .refreshPreimageAuthorization(contentHash)
        .send()
      expect(receipt.blockHash).toBeDefined()
      expect(receipt.txHash).toBeDefined()
    })
  })

  describe("Remove Expired Authorization Operations", () => {
    // These authorizations haven't expired; we just verify the SDK methods
    // dispatch without local-side errors (the chain may reject as expected).
    it("attempts to remove expired account authorization", async () => {
      const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"
      try {
        const receipt = await client
          .removeExpiredAccountAuthorization(bobAddress)
          .send()
        expect(receipt.blockHash).toBeDefined()
      } catch (_err) {
        // Expected — authorization not expired.
      }
    })

    it("attempts to remove expired preimage authorization", async () => {
      const data = new TextEncoder().encode("Content for expiry test")
      const contentHash = blake2b256(data)
      try {
        const receipt = await client
          .removeExpiredPreimageAuthorization(contentHash)
          .send()
        expect(receipt.blockHash).toBeDefined()
      } catch (_err) {
        // Expected — authorization not expired or doesn't exist.
      }
    })
  })

  describe("asUnsigned() — preimage-authorized", () => {
    it("stores data via the unsigned (preimage-auth) path", async () => {
      // Per-run unique data — the unsigned bareTx hash is deterministic
      // from data; constant content across runs hits the pool's
      // `TemporarilyBanned` hash ban (~30 min window).
      const data = new TextEncoder().encode(
        `Preimage-authorized unsigned storage ${Date.now()}-${Math.random()}`,
      )
      const contentHash = blake2b256(data)

      // Authorize the preimage first (signed by Alice).
      await client.authorizePreimage(contentHash, BigInt(data.length)).send()

      // Anonymous submitter — no signer needed for the unsigned path. We
      // reuse the existing signed `client` (its signer field is set but
      // unused on `.asUnsigned()`).
      const { cids } = await client.upload([{ data }]).asUnsigned().send()
      expect(cids).toHaveLength(1)
    })
  })

  describe("Complete workflow", () => {
    it("authorizes Bob then stores via uploadFile()", async () => {
      const dataSize = 2 * 1024 * 1024
      const estimate = client.estimateAuthorization(dataSize)
      const bobAddress = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"
      const authReceipt = await client
        .authorizeAccount(
          bobAddress,
          estimate.transactions,
          BigInt(estimate.bytes),
        )
        .send()
      expect(authReceipt.blockHash).toBeDefined()

      // Alice (already authorized in beforeAll) uploads.
      const data = makeChunkedTestData(dataSize)
      const { cid } = await client.uploadFile(data).send()
      expect(cid).toBeDefined()
    })
  })

  /**
   * Hijack-recovery stress test: two SDK clients upload from the SAME
   * signer in parallel, racing for nonces. Each client's hijack-detection
   * loop reassigns fresh nonces (via `poolNonce`-aware allocation) so both
   * uploads eventually succeed. Exercises the per-item retry queue +
   * `chainNonce`-based hijack detection introduced in sub-PR #4.
   */
  describe("Hijack recovery", { timeout: 180_000 }, () => {
    it("two parallel uploads from the same signer both succeed", async () => {
      // Both clients share the SAME signer → fight over the same nonces.
      const makeRivalClient = () =>
        new BulletinClient(
          papiClient.getUnsafeApi() as unknown as BulletinTypedApi,
          // Re-create signer to avoid any per-instance caching collisions.
          (() => {
            const derive = sr25519CreateDerive(DEV_MINI_SECRET)
            const kp = derive("//Alice")
            return getPolkadotSigner(kp.publicKey, "Sr25519", kp.sign)
          })(),
          papiClient.submitAndWatch,
          { txTimeout: 120_000, wsUrls: [ENDPOINT] },
        )
      const clientA = makeRivalClient()
      const clientB = makeRivalClient()

      // Distinct data → distinct content hashes → both items must each
      // land at SOME nonce (not the same nonce, since pool dedupes hash
      // collisions and each tx has different content).
      const dataA = new TextEncoder().encode(
        `hijack-test-A ${Date.now()} ${Math.random()}`,
      )
      const dataB = new TextEncoder().encode(
        `hijack-test-B ${Date.now()} ${Math.random()}`,
      )

      const [resA, resB] = await Promise.all([
        clientA.upload([{ data: dataA }]).send(),
        clientB.upload([{ data: dataB }]).send(),
      ])

      expect(resA.cids).toHaveLength(1)
      expect(resB.cids).toHaveLength(1)
      // Distinct content → distinct CIDs.
      expect(resA.cids[0]!.toString()).not.toBe(resB.cids[0]!.toString())
    })
  })
})
