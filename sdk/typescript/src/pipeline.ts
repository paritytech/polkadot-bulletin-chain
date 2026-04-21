// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Optimal bulk submission pipeline for Bulletin Chain.
 *
 * Event-driven algorithm that watches best and finalized blocks on one RPC,
 * re-signs a fresh batch of transactions on every best block, and broadcasts
 * each signed tx to **all** RPC endpoints. Completion is gated on finalization.
 *
 * Key properties:
 * - Re-signs per block to bypass pool bans (fresh hashes on each wave)
 * - Mortal transactions (64-block period) so old waves expire
 * - Batch size computed from block weight/length limits
 * - bestNonce assigned directly (not max) to handle reorgs
 * - Finalization-based completion — no false positives from pool nonces
 *
 * @packageDocumentation
 */

import { base58Encode, blake2AsU8a } from "@polkadot/util-crypto"
import type { JsonRpcProvider } from "@polkadot-api/json-rpc-provider"
import {
  createClient as createSubstrateClient,
  type FollowEventWithoutRuntime,
  type FollowResponse,
  type SubstrateClient,
} from "@polkadot-api/substrate-client"
import { Binary, type PolkadotSigner } from "polkadot-api"

import type { BulletinTypedApi } from "./async-client.js"

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/**
 * Block capacity constants for batch computation.
 *
 * These should be determined offline from the chain's runtime constants
 * and pallet benchmarks. See the module-level docs for guidance.
 */
export interface BlockLimits {
  /** Max normal-class weight budget (ref_time) per block. */
  maxNormalWeight: bigint
  /** Max normal-class block length in bytes. */
  normalBlockLength: number
  /** Hard per-block limit on store extrinsics (`TransactionStorage::MaxBlockTransactions`). */
  maxBlockTransactions: number
  /** Base weight of a `store` extrinsic (constant part). */
  storeWeightBase: bigint
  /** Per-byte weight slope of a `store` extrinsic. */
  storeWeightPerByte: bigint
  /** Encoding overhead per extrinsic (signature + address + extensions), ~110 bytes. */
  extrinsicOverhead: number
}

/** Configuration for {@link pipelineStore}. */
export interface PipelineConfig {
  /**
   * RPC WebSocket URLs.
   *
   * Block watching uses the first URL. Every signed transaction is
   * broadcast to **all** URLs so that every node's pool receives the batch.
   */
  wsUrls: string[]
  /** Factory that creates a {@link JsonRpcProvider} from a URL. */
  createProvider: (url: string) => JsonRpcProvider
  /** Block capacity limits for batch computation. */
  blockLimits: BlockLimits
  /** Progress callback fired on each best/finalized block. */
  onProgress?: (stats: PipelineStats) => void
}

/** Snapshot of pipeline progress (emitted via {@link PipelineConfig.onProgress}). */
export interface PipelineStats {
  /** Number of signing waves dispatched so far. */
  waves: number
  /** Number of individual `author_submitExtrinsic` RPC calls. */
  txsBroadcast: number
  /** Number of broadcast errors (all non-fatal). */
  broadcastErrors: number
  /** Confirmed items at best block (`bestNonce - startNonce`; may decrease on reorg). */
  confirmed: number
  /** Finalized items (monotonically increasing, irreversible). */
  finalized: number
  /** Total items to upload. */
  totalItems: number
  /** Elapsed milliseconds since pipeline start. */
  elapsedMs: number
  /** Finalized throughput in tx/s. */
  txPerSec: number
  /** Finalized throughput in bytes/s (based on finalized items' total data size). */
  throughputBytesPerSec: number
}

/** Final result returned by {@link pipelineStore}. */
export interface PipelineResult extends PipelineStats {
  /** Total data bytes across all items. */
  totalBytes: number
  /** Duration in milliseconds. */
  durationMs: number
  /** Starting account nonce (read from finalized block). */
  startNonce: number
  /** Expected final nonce (`startNonce + items.length`). */
  expectedFinalNonce: number
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/** Extended PAPI transaction with the `sign()` method (not in the SDK's minimal interface). */
interface SignableTransaction {
  sign(
    from: PolkadotSigner,
    options?: {
      nonce?: number
      mortality?: { mortal: boolean; period?: number }
    },
  ): Promise<string>
}

// ---------------------------------------------------------------------------
// pipelineStore — main entry point
// ---------------------------------------------------------------------------

/**
 * Submit items through an event-driven pipeline.
 *
 * On each best block:
 * 1. Query `system_accountNextIndex` for the current nonce
 * 2. Compute a batch that fits in one block (weight + length + count)
 * 3. Sign each tx with a 4-block mortal era
 * 4. Broadcast every signed tx to every RPC endpoint
 *
 * Completion: when the account nonce at a finalized block ≥ `startNonce + items.length`.
 */
export async function pipelineStore(
  api: BulletinTypedApi,
  signer: PolkadotSigner,
  items: Uint8Array[],
  config: PipelineConfig,
  signal?: AbortSignal,
): Promise<PipelineResult> {
  if (items.length === 0) return emptyResult()

  const { wsUrls, createProvider, blockLimits, onProgress } = config
  if (wsUrls.length === 0) {
    throw new Error("pipelineStore: at least one wsUrl is required")
  }

  // Hex-encoded pubkey for SCALE state_call (AccountNonceApi)
  const signerHex = hexEncodePublicKey(signer.publicKey)
  // SS58 address for system_accountNextIndex RPC
  const signerSs58 = ss58Encode(signer.publicKey, 42)

  // Pre-compute cumulative byte sizes for throughput reporting
  const prefixBytes = new Float64Array(items.length + 1)
  for (let i = 0; i < items.length; i++) {
    prefixBytes[i + 1] = (prefixBytes[i] ?? 0) + (items[i]?.length ?? 0)
  }
  const totalDataBytes = prefixBytes[items.length] ?? 0

  // ---------------------------------------------------------------------------
  // Connections
  // ---------------------------------------------------------------------------

  // Monitor: one client for block-following + nonce queries
  const monitorClient = createSubstrateClient(
    createProvider(wsUrls[0] as string),
  )

  // Submission: one client per RPC URL (broadcast to all)
  const submitClients = wsUrls.map((url) =>
    createSubstrateClient(createProvider(url)),
  )

  // Abort plumbing
  const ctl = new AbortController()
  if (signal) {
    signal.addEventListener("abort", () => ctl.abort(), { once: true })
  }

  const startTime = Date.now()
  let startNonce = 0
  let expectedFinalNonce = 0
  let initialized = false
  let done = false

  const counters = {
    waves: 0,
    txsBroadcast: 0,
    broadcastErrors: 0,
    confirmed: 0,
    finalized: 0,
  }

  return new Promise<PipelineResult>((resolve, reject) => {
    // -----------------------------------------------------------------
    // Event queue — serializes async processing of chainHead events
    // -----------------------------------------------------------------
    const queue: Array<() => Promise<void>> = []
    let draining = false

    function enqueue(fn: () => Promise<void>): void {
      queue.push(fn)
      if (!draining) drain()
    }

    async function drain(): Promise<void> {
      draining = true
      while (queue.length > 0 && !done && !ctl.signal.aborted) {
        const fn = queue.shift()
        if (!fn) break
        try {
          await fn()
        } catch {
          /* event processing errors are non-fatal */
        }
      }
      draining = false
    }

    function finish(): void {
      if (done) return
      done = true
      try {
        follower.unfollow()
      } catch {
        /* ignore */
      }
      try {
        monitorClient.destroy()
      } catch {
        /* ignore */
      }
      for (const c of submitClients) {
        try {
          c.destroy()
        } catch {
          /* ignore */
        }
      }

      const durationMs = Date.now() - startTime
      const sec = durationMs / 1000
      const finalizedBytes = prefixBytes[counters.finalized] ?? 0
      resolve({
        waves: counters.waves,
        txsBroadcast: counters.txsBroadcast,
        broadcastErrors: counters.broadcastErrors,
        confirmed: counters.confirmed,
        finalized: counters.finalized,
        totalItems: items.length,
        totalBytes: totalDataBytes,
        elapsedMs: durationMs,
        durationMs,
        txPerSec: sec > 0 ? counters.finalized / sec : 0,
        throughputBytesPerSec: sec > 0 ? finalizedBytes / sec : 0,
        startNonce,
        expectedFinalNonce,
      })
    }

    // -----------------------------------------------------------------
    // ChainHead follow — block events drive the state machine
    // -----------------------------------------------------------------
    const follower: FollowResponse = monitorClient.chainHead(
      false,
      (event: FollowEventWithoutRuntime) => {
        if (done || ctl.signal.aborted) return

        switch (event.type) {
          // ---------------------------------------------------------------
          // initialized — read start nonce from the finalized block
          // ---------------------------------------------------------------
          case "initialized": {
            const hashes = event.finalizedBlockHashes
            const lastHash = hashes[hashes.length - 1]
            if (!lastHash) break
            enqueue(async () => {
              startNonce = await readNonceAtBlock(
                monitorClient,
                signerHex,
                lastHash,
              )
              expectedFinalNonce = startNonce + items.length
              initialized = true

              // Unpin initialization blocks (we queried via legacy RPC, not chainHead)
              follower.unpin(hashes).catch(() => {})
            })
            break
          }

          // ---------------------------------------------------------------
          // newBlock — nothing to do, but we must eventually unpin
          // ---------------------------------------------------------------
          case "newBlock":
            // Unpinned in bulk on the next `finalized` event
            break

          // ---------------------------------------------------------------
          // bestBlockChanged — core submission loop
          // ---------------------------------------------------------------
          case "bestBlockChanged": {
            enqueue(async () => {
              if (!initialized || done) return

              // Query nonce — assigned directly, NOT max (reorgs can lower it)
              const bestNonce = await monitorClient.request<number>(
                "system_accountNextIndex",
                [signerSs58],
              )
              counters.confirmed = clamp(
                bestNonce - startNonce,
                0,
                items.length,
              )

              // All items acknowledged at best level — wait for finalization
              if (bestNonce >= expectedFinalNonce) return

              // Compute batch that fits in one block
              const fromIndex = Math.max(0, bestNonce - startNonce)
              const toIndex = computeBatchEnd(items, fromIndex, blockLimits)
              if (fromIndex >= toIndex) return

              // Sign the batch — fresh signatures bypass pool bans
              const signed: string[] = []
              for (let i = fromIndex; i < toIndex; i++) {
                const tx = api.tx.TransactionStorage.store({
                  data: Binary.fromBytes(items[i] as Uint8Array),
                })
                const hex = await (tx as unknown as SignableTransaction).sign(
                  signer,
                  {
                    nonce: startNonce + i,
                    mortality: { mortal: true, period: 64 },
                  },
                )
                signed.push(hex)
              }

              // Broadcast every tx to every RPC
              const promises: Promise<void>[] = []
              for (const hex of signed) {
                for (const client of submitClients) {
                  promises.push(
                    client
                      .request("author_submitExtrinsic", [hex])
                      .then(() => {
                        counters.txsBroadcast++
                      })
                      .catch(() => {
                        counters.broadcastErrors++
                      }),
                  )
                }
              }
              await Promise.allSettled(promises)
              counters.waves++

              if (onProgress) {
                emitProgress(
                  counters,
                  items.length,
                  prefixBytes,
                  startTime,
                  onProgress,
                )
              }
            })
            break
          }

          // ---------------------------------------------------------------
          // finalized — check completion, unpin blocks
          // ---------------------------------------------------------------
          case "finalized": {
            const { finalizedBlockHashes, prunedBlockHashes } = event
            const lastHash =
              finalizedBlockHashes[finalizedBlockHashes.length - 1]
            if (!lastHash) break

            enqueue(async () => {
              // Unpin all reported blocks to avoid hitting the server's pin limit
              const toUnpin = [...finalizedBlockHashes, ...prunedBlockHashes]
              follower.unpin(toUnpin).catch(() => {})

              if (!initialized || done) return

              const finNonce = await readNonceAtBlock(
                monitorClient,
                signerHex,
                lastHash,
              )
              counters.finalized = clamp(finNonce - startNonce, 0, items.length)

              if (onProgress) {
                emitProgress(
                  counters,
                  items.length,
                  prefixBytes,
                  startTime,
                  onProgress,
                )
              }

              if (finNonce >= expectedFinalNonce) {
                finish()
              }
            })
            break
          }
        }
      },
      (error) => {
        if (!done) reject(error)
      },
    )

    // Handle external abort
    ctl.signal.addEventListener(
      "abort",
      () => {
        if (!done) finish()
      },
      { once: true },
    )
  })
}

// ---------------------------------------------------------------------------
// Batch computation
// ---------------------------------------------------------------------------

/**
 * Pack payloads into a batch that fits in one block.
 *
 * Iterates from `fromIndex`, accumulating each tx's weight and length
 * contribution, and stops when any block limit would be exceeded.
 */
function computeBatchEnd(
  items: Uint8Array[],
  fromIndex: number,
  limits: BlockLimits,
): number {
  let toIndex = fromIndex
  let accWeight = 0n
  let accLength = 0

  while (toIndex < items.length) {
    const size = items[toIndex]?.length ?? 0
    const txWeight =
      limits.storeWeightBase + limits.storeWeightPerByte * BigInt(size)
    const txLength = size + limits.extrinsicOverhead

    if (accWeight + txWeight > limits.maxNormalWeight) break
    if (accLength + txLength > limits.normalBlockLength) break
    if (toIndex - fromIndex >= limits.maxBlockTransactions) break

    accWeight += txWeight
    accLength += txLength
    toIndex++
  }

  return toIndex
}

// ---------------------------------------------------------------------------
// Nonce reading
// ---------------------------------------------------------------------------

/**
 * Read the account nonce at a specific block via `AccountNonceApi`.
 *
 * Uses the legacy `state_call` RPC which accepts a block hash parameter.
 * This avoids reading `System::Account` storage directly and works on
 * all Polkadot SDK nodes with the `AccountNonceApi` runtime API.
 */
async function readNonceAtBlock(
  client: SubstrateClient,
  accountHex: string,
  blockHash: string,
): Promise<number> {
  const resultHex = await client.request<string>("state_call", [
    "AccountNonceApi_account_nonce",
    accountHex,
    blockHash,
  ])
  return decodeU32LE(resultHex)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Decode a SCALE-encoded u32 (4 bytes, little-endian) from a hex string. */
function decodeU32LE(hex: string): number {
  const h = hex.startsWith("0x") ? hex.slice(2) : hex
  return (
    (parseInt(h.slice(0, 2), 16) |
      (parseInt(h.slice(2, 4), 16) << 8) |
      (parseInt(h.slice(4, 6), 16) << 16) |
      (parseInt(h.slice(6, 8), 16) << 24)) >>>
    0
  )
}

/** Encode a 32-byte public key as SS58 address for RPC calls like system_accountNextIndex. */
function ss58Encode(pubKey: Uint8Array, prefix: number): string {
  // SS58 for simple prefixes (0-63): [prefix(1), pubkey(32), checksum(2)]
  const payload = new Uint8Array(35)
  payload[0] = prefix
  payload.set(pubKey, 1)
  // Checksum = first 2 bytes of blake2b-512("SS58PRE" || prefix || pubkey)
  const SS58_PREFIX = new TextEncoder().encode("SS58PRE")
  const input = new Uint8Array(SS58_PREFIX.length + 33) // 7 + 1 + 32 = 40
  input.set(SS58_PREFIX)
  input.set(payload.subarray(0, 33), SS58_PREFIX.length)
  const hash = blake2AsU8a(input, 512)
  payload[33] = hash[0] ?? 0
  payload[34] = hash[1] ?? 0
  return base58Encode(payload)
}

/** Hex-encode a 32-byte public key as `0x...` for RPC calls. */
function hexEncodePublicKey(pubKey: Uint8Array): string {
  return (
    "0x" +
    Array.from(pubKey)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("")
  )
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value))
}

function emitProgress(
  counters: {
    waves: number
    txsBroadcast: number
    broadcastErrors: number
    confirmed: number
    finalized: number
  },
  totalItems: number,
  prefixBytes: Float64Array,
  startTime: number,
  cb: (stats: PipelineStats) => void,
): void {
  const elapsedMs = Date.now() - startTime
  const sec = elapsedMs / 1000
  const finalizedBytes = prefixBytes[counters.finalized] ?? 0
  cb({
    waves: counters.waves,
    txsBroadcast: counters.txsBroadcast,
    broadcastErrors: counters.broadcastErrors,
    confirmed: counters.confirmed,
    finalized: counters.finalized,
    totalItems,
    elapsedMs,
    txPerSec: sec > 0 ? counters.finalized / sec : 0,
    throughputBytesPerSec: sec > 0 ? finalizedBytes / sec : 0,
  })
}

function emptyResult(): PipelineResult {
  return {
    waves: 0,
    txsBroadcast: 0,
    broadcastErrors: 0,
    confirmed: 0,
    finalized: 0,
    totalItems: 0,
    totalBytes: 0,
    elapsedMs: 0,
    durationMs: 0,
    txPerSec: 0,
    throughputBytesPerSec: 0,
    startNonce: 0,
    expectedFinalNonce: 0,
  }
}
