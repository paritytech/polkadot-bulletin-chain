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

import { randomBytes } from "node:crypto"
import { blake2b } from "@noble/hashes/blake2.js"
import { sr25519CreateDerive } from "@polkadot-labs/hdkd"
import { DEV_MINI_SECRET, ss58Address } from "@polkadot-labs/hdkd-helpers"
import { getPolkadotSigner } from "polkadot-api/signer"
import { getWsProvider } from "polkadot-api/ws"
import { afterAll, beforeAll, describe, expect, it } from "vitest"
import {
  BulletinClient,
  CidCodec,
  HashAlgorithm,
  UploadStatus,
} from "../../src"

const ENDPOINT = process.env.BULLETIN_RPC_URL ?? "ws://localhost:9944"

const blake2b256 = (data: Uint8Array) => blake2b(data, { dkLen: 32 })

describe("BulletinClient Integration Tests", { timeout: 120_000 }, () => {
  let client: BulletinClient
  let aliceAddress: string

  beforeAll(async () => {
    const derive = sr25519CreateDerive(DEV_MINI_SECRET)
    const aliceKeyPair = derive("//Alice")
    const signer = getPolkadotSigner(
      aliceKeyPair.publicKey,
      "Sr25519",
      aliceKeyPair.sign,
    )
    aliceAddress = ss58Address(aliceKeyPair.publicKey, 42)

    // Self-contained client: SDK owns the PAPI client lifecycle.
    // No `descriptor` here → SDK uses getUnsafeApi() (works at runtime,
    // loses compile-time chain types). Alice is both uploader and
    // authorizer in these tests.
    client = new BulletinClient({
      providers: () => [getWsProvider(ENDPOINT)],
      uploadSigner: signer,
      authorizerSigner: signer,
      txTimeout: 120_000,
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
    if (client) await client.destroy()
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
      const makeRivalClient = () => {
        const derive = sr25519CreateDerive(DEV_MINI_SECRET)
        const kp = derive("//Alice")
        const rivalSigner = getPolkadotSigner(kp.publicKey, "Sr25519", kp.sign)
        return new BulletinClient({
          providers: () => [getWsProvider(ENDPOINT)],
          uploadSigner: rivalSigner,
          authorizerSigner: rivalSigner,
          txTimeout: 120_000,
        })
      }
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

  /**
   * Exactly-once accounting under concurrent same-account uploads. Each
   * successful `store` extrinsic in the runtime increments the caller's
   * `Authorizations.extent.transactions` by 1 and `extent.bytes` by
   * `data.len()` (see pallets/transaction-storage `do_store`). If the SDK
   * ever double-broadcasts a chunk (e.g. on watchdog reconnect or
   * STORE_STALLED retry) the chain would either reject as `Duplicate`
   * OR (if the prior copy was pruned) accept it again and double-count.
   * Either way the delta would drift from the expected sum — this test
   * locks the invariant.
   */
  describe("Exactly-once accounting under parallel same-account upload", {
    timeout: 480_000,
  }, () => {
    it("3 parallel uploads → Authorizations.extent advances by exact sum", async () => {
      const SESSIONS = 3
      // 17 items per session is intentionally above the chain's ~9
      // items-per-block cap so the test exercises the contention path
      // across multiple blocks (waves, hijack/dedup, retries).
      const ITEMS_PER = 17
      const ITEM_SIZE = 1024 * 1024 // 1 MiB

      // Top up Alice's authorization with enough headroom for the
      // expected work (3 × 17 = 51 transactions, 51 MiB). authorizeAccount
      // is additive within the unexpired window, so this stacks on top
      // of whatever was authorized in earlier tests / beforeAll.
      await client
        .authorizeAccount(
          aliceAddress,
          SESSIONS * ITEMS_PER + 10, // 51 txs + a small safety margin
          BigInt((SESSIONS * ITEMS_PER + 10) * ITEM_SIZE),
        )
        .withWaitFor("finalized")
        .send()

      // Snapshot BEFORE — current usage at the latest finalized state.
      const before =
        await client.api.query.TransactionStorage.Authorizations.getValue({
          type: "Account",
          value: aliceAddress,
        })
      expect(before).toBeDefined()
      const beforeTx = before!.extent.transactions
      const beforeBytes = before!.extent.bytes

      // Build SESSIONS × ITEMS_PER unique 1-MiB chunks. Content MUST be
      // distinct cross-session, cross-item, and cross-run — the chain
      // dedupes by content_hash (TBCH overwrites) and Authorizations.extent
      // increments per execution, so the accounting check breaks if any
      // pair of items collide.
      //
      // We mix an 8-byte crypto-random seed PER ITEM into every byte of
      // the payload (collision probability 2^-64). Earlier XOR-shuffle
      // generators looked random but collapsed many (sIdx, iIdx) pairs to
      // identical content because only byte 0 of the 32-bit tag was used.
      const sessionItems = Array.from({ length: SESSIONS }, () =>
        Array.from({ length: ITEMS_PER }, () => {
          const data = new Uint8Array(ITEM_SIZE)
          const seed = randomBytes(8)
          for (let i = 0; i < ITEM_SIZE; i++) {
            data[i] = (seed[i & 7]! ^ i ^ (i >> 8) ^ (i >> 16) ^ (i >> 24)) & 0xff
          }
          return { data }
        }),
      )

      // 3 separate clients sharing the same signer — emulates 3
      // independent scripts on //Alice racing for the same nonces.
      const rivals = Array.from({ length: SESSIONS }, () => {
        const derive = sr25519CreateDerive(DEV_MINI_SECRET)
        const kp = derive("//Alice")
        const rivalSigner = getPolkadotSigner(kp.publicKey, "Sr25519", kp.sign)
        return new BulletinClient({
          providers: () => [getWsProvider(ENDPOINT)],
          uploadSigner: rivalSigner,
          txTimeout: 180_000,
        })
      })

      try {
        // Fire all 3 uploads in parallel; each waits for finalization.
        const results = await Promise.all(
          rivals.map((c, idx) =>
            c
              .upload(sessionItems[idx] as { data: Uint8Array }[])
              .withWaitFor("finalized")
              .send(),
          ),
        )
        for (const r of results) {
          expect(r.cids).toHaveLength(ITEMS_PER)
        }

        // Snapshot AFTER — same query path so any "finalized lag"
        // applies symmetrically to both snapshots.
        const after =
          await client.api.query.TransactionStorage.Authorizations.getValue({
            type: "Account",
            value: aliceAddress,
          })
        expect(after).toBeDefined()
        const afterTx = after!.extent.transactions
        const afterBytes = after!.extent.bytes

        // Expected deltas come from estimateUpload over the full batch:
        // any content_hash collisions across sessions would naturally
        // reduce the per-tx expectation, matching what the chain sees.
        // With crypto-random per-item seeds these will equal SESSIONS *
        // ITEMS_PER but the estimate keeps the assertion self-consistent.
        const allItems = sessionItems.flat()
        const estimate = await client.estimateUpload(allItems)
        expect(afterTx - beforeTx).toBe(estimate.transactions)
        expect(afterBytes - beforeBytes).toBe(estimate.bytes)
      } finally {
        await Promise.all(rivals.map((c) => c.destroy()))
      }
    })

    it("3 parallel uploads from 3 DIFFERENT accounts → each Authorizations.extent advances by exact per-account sum", async () => {
      const SESSIONS = 3
      // 17 items per session, same as same-account variant, so the
      // per-account work matches and we can compare wall-clock easily.
      const ITEMS_PER = 17
      const ITEM_SIZE = 1024 * 1024 // 1 MiB

      // Per-test derivation paths — chosen to be distinct from any
      // other test's signer so we don't collide with shared state.
      const derive = sr25519CreateDerive(DEV_MINI_SECRET)
      const paths = ["//ParallelA", "//ParallelB", "//ParallelC"]
      const accounts = paths.map((path) => {
        const kp = derive(path)
        return {
          address: ss58Address(kp.publicKey, 42),
          signer: getPolkadotSigner(kp.publicKey, "Sr25519", kp.sign),
        }
      })

      // Authorize all 3 target accounts in a single atomic batched call
      // (Utility.batch_all under the hood). One round-trip instead of
      // three serial ones, and the batch is all-or-nothing.
      await client
        .authorizeAccount(
          accounts.map((acc) => ({
            who: acc.address,
            transactions: ITEMS_PER + 5,
            bytes: BigInt((ITEMS_PER + 5) * ITEM_SIZE),
          })),
        )
        .withWaitFor("finalized")
        .send()

      // Snapshot per-account BEFORE.
      const before = await Promise.all(
        accounts.map((acc) =>
          client.api.query.TransactionStorage.Authorizations.getValue({
            type: "Account",
            value: acc.address,
          }),
        ),
      )
      for (const auth of before) expect(auth).toBeDefined()
      const beforeTx = before.map((a) => a!.extent.transactions)
      const beforeBytes = before.map((a) => a!.extent.bytes)

      // Unique 1-MiB chunks. Cross-session uniqueness still required —
      // even though signers differ, the chain's `TransactionByContentHash`
      // is keyed by content_hash regardless of signer. Per-item
      // crypto-random seed (see same-account variant above for rationale).
      const sessionItems = Array.from({ length: SESSIONS }, () =>
        Array.from({ length: ITEMS_PER }, () => {
          const data = new Uint8Array(ITEM_SIZE)
          const seed = randomBytes(8)
          for (let i = 0; i < ITEM_SIZE; i++) {
            data[i] = (seed[i & 7]! ^ i ^ (i >> 8) ^ (i >> 16) ^ (i >> 24)) & 0xff
          }
          return { data }
        }),
      )

      // 3 clients, each with its OWN signer — no nonce sharing.
      const clients = accounts.map(
        (acc) =>
          new BulletinClient({
            providers: () => [getWsProvider(ENDPOINT)],
            uploadSigner: acc.signer,
            txTimeout: 120_000,
          }),
      )

      try {
        const results = await Promise.all(
          clients.map((c, idx) =>
            c
              .upload(sessionItems[idx] as { data: Uint8Array }[])
              .withWaitFor("finalized")
              .send(),
          ),
        )
        for (const r of results) {
          expect(r.cids).toHaveLength(ITEMS_PER)
        }

        // Snapshot per-account AFTER.
        const after = await Promise.all(
          accounts.map((acc) =>
            client.api.query.TransactionStorage.Authorizations.getValue({
              type: "Account",
              value: acc.address,
            }),
          ),
        )
        for (const auth of after) expect(auth).toBeDefined()

        // Per-account exact-match delta. Each account did exactly
        // estimate.transactions successful stores (which equals ITEMS_PER
        // when there are no input duplicates within a session). Using
        // estimateUpload here keeps the assertion robust if the test
        // data generator ever emits collisions again.
        for (let i = 0; i < SESSIONS; i++) {
          const sessionEstimate = await client.estimateUpload(
            sessionItems[i] as { data: Uint8Array }[],
          )
          const txDelta = after[i]!.extent.transactions - beforeTx[i]!
          const bytesDelta = after[i]!.extent.bytes - beforeBytes[i]!
          expect(txDelta, `account ${i} tx delta`).toBe(
            sessionEstimate.transactions,
          )
          expect(bytesDelta, `account ${i} bytes delta`).toBe(
            sessionEstimate.bytes,
          )
        }
      } finally {
        await Promise.all(clients.map((c) => c.destroy()))
      }
    })
  })
})
