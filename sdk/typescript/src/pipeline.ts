// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Optimal bulk submission pipeline for Bulletin Chain.
 *
 * Event-driven algorithm that watches best and finalized blocks on one RPC,
 * speculatively pre-signs the next batch while the current wave propagates,
 * and submits to a single RPC (gossip distributes to the rest).
 *
 * Key properties:
 * - Speculative pre-signing: signs the next batch concurrently with the
 *   current broadcast. On the next block, if prediction holds, just
 *   broadcast (sub-second); if not, re-sign.
 * - Broadcasts to all RPC endpoints for maximum propagation
 * - Mortal transactions (64-block period) so waves eventually expire
 * - Batch size computed from block weight/length limits
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
import { Binary, getOfflineApi, type PolkadotSigner } from "polkadot-api"

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
  /**
   * Raw signing function for fast-path signing.
   *
   * When provided, the pipeline bypasses PAPI's per-tx metadata decode
   * (which costs ~100ms per tx) and signs transactions directly.
   * Pass the `sign` function from your keypair (e.g. `keyPair.sign`).
   */
  rawSign?: (message: Uint8Array) => Promise<Uint8Array>
  /** Signing type. Required when `rawSign` is provided. Default: `"Sr25519"`. */
  signingType?: "Sr25519" | "Ed25519" | "Ecdsa"
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

/** Latency distribution in milliseconds. */
export interface LatencyStats {
  count: number
  min: number
  max: number
  mean: number
  p50: number
  p90: number
  p95: number
  p99: number
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
  /**
   * Per-item inclusion latency: time from first broadcast of an item's tx
   * to the moment its nonce was observed at a best block. Null if no items
   * were observed at best block.
   */
  inclusionLatency: LatencyStats | null
  /**
   * Per-item finalization latency: time from first broadcast of an item's tx
   * to the moment its nonce was observed at a finalized block. Null if no
   * items were finalized.
   */
  finalizationLatency: LatencyStats | null
  /** Raw per-item inclusion latencies (ms). Indexed by item offset. */
  inclusionLatenciesMs: number[]
  /** Raw per-item finalization latencies (ms). Indexed by item offset. */
  finalizationLatenciesMs: number[]
}

/** Signer descriptor for multi-account pipeline. */
export interface MultiAccountSigner {
  /** PAPI signer. */
  signer: PolkadotSigner
  /** Raw signing function for fast-path (same as {@link PipelineConfig.rawSign}). */
  rawSign?: (message: Uint8Array) => Promise<Uint8Array>
}

/** Aggregated result from {@link pipelineStoreMulti}. */
export interface MultiPipelineResult {
  /** Number of accounts used. */
  accounts: number
  /** Per-account results. */
  perAccount: PipelineResult[]
  /** Total items across all accounts. */
  totalItems: number
  /** Total data bytes across all accounts. */
  totalBytes: number
  /** Wall-clock duration in milliseconds. */
  durationMs: number
  /** Total finalized items. */
  finalized: number
  /** Aggregate finalized throughput in tx/s. */
  txPerSec: number
  /** Aggregate finalized throughput in bytes/s. */
  throughputBytesPerSec: number
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/** Offline transaction entry returned by the offline API. */
type OfflineStoreTx = (args: { data: Binary }) => {
  sign(
    from: PolkadotSigner,
    extensions: {
      nonce: number
      mortality:
        | {
            mortal: true
            period: number
            startAtBlock: { height: number; hash: string }
          }
        | { mortal: false }
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
 * 3. Sign each tx with a short mortal era via pre-cached offline API
 * 4. Broadcast every signed tx to every RPC endpoint
 *
 * Completion: when the account nonce at a finalized block ≥ `startNonce + items.length`.
 */
export async function pipelineStore(
  _api: BulletinTypedApi,
  signer: PolkadotSigner,
  items: Uint8Array[],
  config: PipelineConfig,
  signal?: AbortSignal,
): Promise<PipelineResult> {
  if (items.length === 0) return emptyResult()

  const { wsUrls, createProvider, blockLimits, onProgress, rawSign } = config
  const signingType = config.signingType ?? "Sr25519"
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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let offlineStoreTx: OfflineStoreTx = null as any
  let effectiveSigner: PolkadotSigner = signer

  // Speculative pre-signing state
  let preSigned: { fromNonce: number; hexes: string[] } | null = null

  const counters = {
    waves: 0,
    txsBroadcast: 0,
    broadcastErrors: 0,
    confirmed: 0,
    finalized: 0,
  }

  // First-broadcast timestamp per item offset (ms since epoch).
  // Undefined entries mean the item has not yet been broadcast.
  const broadcastAtMs: Array<number | undefined> = new Array(items.length)
  // Latency arrays indexed by item offset (0..items.length).
  const inclusionLatenciesMs: number[] = []
  const finalizationLatenciesMs: number[] = []
  // High-water marks: we have already recorded latency for offsets [0, mark).
  let inclusionRecordedTo = 0
  let finalizationRecordedTo = 0

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
        inclusionLatency: computeLatencyStats(inclusionLatenciesMs),
        finalizationLatency: computeLatencyStats(finalizationLatenciesMs),
        inclusionLatenciesMs: [...inclusionLatenciesMs],
        finalizationLatenciesMs: [...finalizationLatenciesMs],
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
              // Fetch start nonce, genesis hash, and metadata in parallel
              const [nonce, genesisHash, metadataHex] = await Promise.all([
                readNonceAtBlock(monitorClient, signerHex, lastHash),
                monitorClient.request<string>("chain_getBlockHash", [0]),
                monitorClient.request<string>("state_getMetadata", []),
              ])
              startNonce = nonce
              expectedFinalNonce = startNonce + items.length

              // Build offline API — metadata decoded once, reused for all signing
              const metadataRaw = hexToBytes(metadataHex)
              const offlineApi = await (
                getOfflineApi as (opts: {
                  genesis: string
                  getMetadata: () => Promise<Uint8Array>
                  // eslint-disable-next-line @typescript-eslint/no-explicit-any
                }) => Promise<any>
              )({
                genesis: genesisHash,
                getMetadata: async () => metadataRaw,
              })
              offlineStoreTx = offlineApi.tx.TransactionStorage
                .store as OfflineStoreTx

              // Build fast-path signer (bypasses per-tx metadata decode)
              if (rawSign) {
                effectiveSigner = await createFastSigner(
                  rawSign,
                  signer.publicKey,
                  signingType,
                  metadataRaw,
                )
              }

              initialized = true
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
            const bestBlockHash = (
              event as { type: "bestBlockChanged"; bestBlockHash: string }
            ).bestBlockHash
            enqueue(async () => {
              if (!initialized || done) return

              // Query nonce and block header in parallel
              const [bestNonce, header] = await Promise.all([
                monitorClient.request<number>("system_accountNextIndex", [
                  signerSs58,
                ]),
                monitorClient.request<{ number: string }>("chain_getHeader", [
                  bestBlockHash,
                ]),
              ])
              const bestBlockNumber = parseInt(header.number, 16)
              counters.confirmed = clamp(
                bestNonce - startNonce,
                0,
                items.length,
              )

              // Record inclusion latency for newly-confirmed items
              if (counters.confirmed > inclusionRecordedTo) {
                const observedAt = Date.now()
                for (let i = inclusionRecordedTo; i < counters.confirmed; i++) {
                  const broadcast = broadcastAtMs[i]
                  if (broadcast !== undefined) {
                    inclusionLatenciesMs.push(observedAt - broadcast)
                  }
                }
                inclusionRecordedTo = counters.confirmed
              }

              if (bestNonce >= expectedFinalNonce) return

              const fromIndex = Math.max(0, bestNonce - startNonce)
              const toIndex = computeBatchEnd(items, fromIndex, blockLimits)
              if (fromIndex >= toIndex) return

              // Use pre-signed batch if the nonce prediction was correct
              let signed: string[]
              if (preSigned && preSigned.fromNonce === startNonce + fromIndex) {
                signed = preSigned.hexes
                preSigned = null
              } else {
                // Prediction missed (reorg / first wave) — sign now
                preSigned = null
                signed = await signBatch(
                  offlineStoreTx,
                  effectiveSigner,
                  items,
                  fromIndex,
                  toIndex,
                  startNonce,
                  bestBlockNumber,
                  bestBlockHash,
                )
              }

              // Record first-broadcast timestamp for each item in the batch
              const broadcastNow = Date.now()
              for (let i = fromIndex; i < toIndex; i++) {
                if (broadcastAtMs[i] === undefined)
                  broadcastAtMs[i] = broadcastNow
              }

              // Broadcast to all RPCs; pre-sign next batch concurrently
              const broadcastPromises: Promise<void>[] = []
              for (const hex of signed) {
                for (const client of submitClients) {
                  broadcastPromises.push(
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

              // Pre-sign the next batch while broadcast is in flight
              const nextFromIndex = toIndex
              let preSignPromise: Promise<void> = Promise.resolve()
              if (nextFromIndex < items.length) {
                const nextToIndex = computeBatchEnd(
                  items,
                  nextFromIndex,
                  blockLimits,
                )
                if (nextFromIndex < nextToIndex) {
                  preSignPromise = signBatch(
                    offlineStoreTx,
                    effectiveSigner,
                    items,
                    nextFromIndex,
                    nextToIndex,
                    startNonce,
                    bestBlockNumber,
                    bestBlockHash,
                  ).then((nextSigned) => {
                    preSigned = {
                      fromNonce: startNonce + nextFromIndex,
                      hexes: nextSigned,
                    }
                  })
                }
              }

              await Promise.all([
                Promise.allSettled(broadcastPromises),
                preSignPromise,
              ])
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

              // Record finalization latency for newly-finalized items
              if (counters.finalized > finalizationRecordedTo) {
                const observedAt = Date.now()
                for (
                  let i = finalizationRecordedTo;
                  i < counters.finalized;
                  i++
                ) {
                  const broadcast = broadcastAtMs[i]
                  if (broadcast !== undefined) {
                    finalizationLatenciesMs.push(observedAt - broadcast)
                  }
                }
                finalizationRecordedTo = counters.finalized
              }

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
// pipelineStoreMulti — shared-monitor multi-account submission
// ---------------------------------------------------------------------------

/**
 * Submit items using multiple accounts in parallel.
 *
 * Uses a **single** chainHead subscription shared across all accounts.
 * On each best block, signs and broadcasts batches for every account
 * concurrently. This avoids the WS connection overload that occurs when
 * running N independent {@link pipelineStore} instances.
 *
 * Callers must authorize every account **before** calling this function.
 */
export async function pipelineStoreMulti(
  _api: BulletinTypedApi,
  signers: MultiAccountSigner[],
  items: Uint8Array[],
  config: PipelineConfig,
  signal?: AbortSignal,
): Promise<MultiPipelineResult> {
  const n = signers.length
  if (n === 0) throw new Error("pipelineStoreMulti: at least one signer")
  if (items.length === 0) {
    return {
      accounts: n,
      perAccount: [],
      totalItems: 0,
      totalBytes: 0,
      durationMs: 0,
      finalized: 0,
      txPerSec: 0,
      throughputBytesPerSec: 0,
    }
  }

  const { wsUrls, createProvider, blockLimits, onProgress, rawSign } = config
  const signingType = config.signingType ?? "Sr25519"
  if (wsUrls.length === 0) {
    throw new Error("pipelineStoreMulti: at least one wsUrl is required")
  }

  // Split items round-robin across accounts
  const perAcct: Uint8Array[][] = Array.from({ length: n }, () => [])
  for (let i = 0; i < items.length; i++) {
    perAcct[i % n]?.push(items[i]!)
  }

  // Per-account state
  interface AcctState {
    items: Uint8Array[]
    signerHex: string
    signerSs58: string
    signer: PolkadotSigner
    effectiveSigner: PolkadotSigner
    prefixBytes: Float64Array
    startNonce: number
    expectedFinalNonce: number
    confirmed: number
    finalized: number
    waves: number
    txsBroadcast: number
    broadcastErrors: number
    preSigned: { fromNonce: number; hexes: string[] } | null
  }

  const accounts: AcctState[] = signers.map((s, i) => {
    const acctItems = perAcct[i]!
    const pb = new Float64Array(acctItems.length + 1)
    for (let j = 0; j < acctItems.length; j++) {
      pb[j + 1] = (pb[j] ?? 0) + (acctItems[j]?.length ?? 0)
    }
    return {
      items: acctItems,
      signerHex: hexEncodePublicKey(s.signer.publicKey),
      signerSs58: ss58Encode(s.signer.publicKey, 42),
      signer: s.signer,
      effectiveSigner: s.signer,
      prefixBytes: pb,
      startNonce: 0,
      expectedFinalNonce: 0,
      confirmed: 0,
      finalized: 0,
      waves: 0,
      txsBroadcast: 0,
      broadcastErrors: 0,
      preSigned: null,
    }
  })

  // Single monitor client + shared submit clients
  const monitorClient = createSubstrateClient(
    createProvider(wsUrls[0] as string),
  )
  const submitClients = wsUrls.map((url) =>
    createSubstrateClient(createProvider(url)),
  )

  const ctl = new AbortController()
  if (signal) {
    signal.addEventListener("abort", () => ctl.abort(), { once: true })
  }

  const startTime = Date.now()
  let initialized = false
  let done = false
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let offlineStoreTx: OfflineStoreTx = null as any

  return new Promise<MultiPipelineResult>((resolve, reject) => {
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
          /* non-fatal */
        }
      }
      draining = false
    }

    function allFinalized(): boolean {
      return accounts.every(
        (a) => a.finalized >= a.items.length || a.items.length === 0,
      )
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
      const totalItems = accounts.reduce((s, a) => s + a.items.length, 0)
      const totalBytes = accounts.reduce(
        (s, a) => s + (a.prefixBytes[a.items.length] ?? 0),
        0,
      )
      const finalized = accounts.reduce((s, a) => s + a.finalized, 0)
      const finalizedBytes = accounts.reduce(
        (s, a) => s + (a.prefixBytes[a.finalized] ?? 0),
        0,
      )

      const perAccount: PipelineResult[] = accounts.map((a) => {
        const aBytes = a.prefixBytes[a.finalized] ?? 0
        return {
          waves: a.waves,
          txsBroadcast: a.txsBroadcast,
          broadcastErrors: a.broadcastErrors,
          confirmed: a.confirmed,
          finalized: a.finalized,
          totalItems: a.items.length,
          totalBytes: a.prefixBytes[a.items.length] ?? 0,
          elapsedMs: durationMs,
          durationMs,
          txPerSec: sec > 0 ? a.finalized / sec : 0,
          throughputBytesPerSec: sec > 0 ? aBytes / sec : 0,
          startNonce: a.startNonce,
          expectedFinalNonce: a.expectedFinalNonce,
        }
      })

      resolve({
        accounts: n,
        perAccount,
        totalItems,
        totalBytes,
        durationMs,
        finalized,
        txPerSec: sec > 0 ? finalized / sec : 0,
        throughputBytesPerSec: sec > 0 ? finalizedBytes / sec : 0,
      })
    }

    // Single chainHead subscription drives all accounts
    const follower: FollowResponse = monitorClient.chainHead(
      false,
      (event: FollowEventWithoutRuntime) => {
        if (done || ctl.signal.aborted) return

        switch (event.type) {
          case "initialized": {
            const hashes = event.finalizedBlockHashes
            const lastHash = hashes[hashes.length - 1]
            if (!lastHash) break
            enqueue(async () => {
              // Read start nonces for all accounts + genesis + metadata
              const noncePromises = accounts.map((a) =>
                readNonceAtBlock(monitorClient, a.signerHex, lastHash),
              )
              const [genesisHash, metadataHex, ...nonces] = await Promise.all([
                monitorClient.request<string>("chain_getBlockHash", [0]),
                monitorClient.request<string>("state_getMetadata", []),
                ...noncePromises,
              ])

              for (let i = 0; i < accounts.length; i++) {
                const a = accounts[i]!
                a.startNonce = nonces[i]!
                a.expectedFinalNonce = a.startNonce + a.items.length
              }

              // Build offline API once
              const metadataRaw = hexToBytes(metadataHex)
              const offlineApi = await (
                getOfflineApi as (opts: {
                  genesis: string
                  getMetadata: () => Promise<Uint8Array>
                  // eslint-disable-next-line @typescript-eslint/no-explicit-any
                }) => Promise<any>
              )({
                genesis: genesisHash,
                getMetadata: async () => metadataRaw,
              })
              offlineStoreTx = offlineApi.tx.TransactionStorage
                .store as OfflineStoreTx

              // Build fast signers for all accounts
              await Promise.all(
                accounts.map(async (a, i) => {
                  const acctRawSign = signers[i]?.rawSign ?? rawSign
                  if (acctRawSign) {
                    a.effectiveSigner = await createFastSigner(
                      acctRawSign,
                      a.signer.publicKey,
                      signingType,
                      metadataRaw,
                    )
                  }
                }),
              )

              initialized = true
              follower.unpin(hashes).catch(() => {})
            })
            break
          }

          case "newBlock":
            break

          case "bestBlockChanged": {
            const bestBlockHash = (
              event as { type: "bestBlockChanged"; bestBlockHash: string }
            ).bestBlockHash
            enqueue(async () => {
              if (!initialized || done) return

              // Query header + all account nonces in parallel
              const [header, ...bestNonces] = await Promise.all([
                monitorClient.request<{ number: string }>("chain_getHeader", [
                  bestBlockHash,
                ]),
                ...accounts.map((a) =>
                  monitorClient.request<number>("system_accountNextIndex", [
                    a.signerSs58,
                  ]),
                ),
              ])
              const bestBlockNumber = parseInt(header.number, 16)

              // Sign and broadcast for each account concurrently
              const acctWork = accounts.map(async (a, idx) => {
                const bestNonce = bestNonces[idx]!
                a.confirmed = clamp(bestNonce - a.startNonce, 0, a.items.length)
                if (bestNonce >= a.expectedFinalNonce) return

                const fromIndex = Math.max(0, bestNonce - a.startNonce)
                const toIndex = computeBatchEnd(a.items, fromIndex, blockLimits)
                if (fromIndex >= toIndex) return

                // Use pre-signed batch if prediction holds
                let signed: string[]
                if (
                  a.preSigned &&
                  a.preSigned.fromNonce === a.startNonce + fromIndex
                ) {
                  signed = a.preSigned.hexes
                  a.preSigned = null
                } else {
                  a.preSigned = null
                  signed = await signBatch(
                    offlineStoreTx,
                    a.effectiveSigner,
                    a.items,
                    fromIndex,
                    toIndex,
                    a.startNonce,
                    bestBlockNumber,
                    bestBlockHash,
                  )
                }

                // Broadcast
                const broadcasts: Promise<void>[] = []
                for (const hex of signed) {
                  for (const client of submitClients) {
                    broadcasts.push(
                      client
                        .request("author_submitExtrinsic", [hex])
                        .then(() => {
                          a.txsBroadcast++
                        })
                        .catch(() => {
                          a.broadcastErrors++
                        }),
                    )
                  }
                }

                // Pre-sign next batch
                const nextFrom = toIndex
                let preSignP: Promise<void> = Promise.resolve()
                if (nextFrom < a.items.length) {
                  const nextTo = computeBatchEnd(a.items, nextFrom, blockLimits)
                  if (nextFrom < nextTo) {
                    preSignP = signBatch(
                      offlineStoreTx,
                      a.effectiveSigner,
                      a.items,
                      nextFrom,
                      nextTo,
                      a.startNonce,
                      bestBlockNumber,
                      bestBlockHash,
                    ).then((nextSigned) => {
                      a.preSigned = {
                        fromNonce: a.startNonce + nextFrom,
                        hexes: nextSigned,
                      }
                    })
                  }
                }

                await Promise.all([Promise.allSettled(broadcasts), preSignP])
                a.waves++
              })

              await Promise.all(acctWork)

              if (onProgress) {
                const elapsedMs = Date.now() - startTime
                const sec = elapsedMs / 1000
                const finalized = accounts.reduce((s, a) => s + a.finalized, 0)
                const confirmed = accounts.reduce((s, a) => s + a.confirmed, 0)
                const totalItems = accounts.reduce(
                  (s, a) => s + a.items.length,
                  0,
                )
                const waves = Math.max(...accounts.map((a) => a.waves))
                const txsBroadcast = accounts.reduce(
                  (s, a) => s + a.txsBroadcast,
                  0,
                )
                const broadcastErrors = accounts.reduce(
                  (s, a) => s + a.broadcastErrors,
                  0,
                )
                const finalizedBytes = accounts.reduce(
                  (s, a) => s + (a.prefixBytes[a.finalized] ?? 0),
                  0,
                )
                onProgress({
                  waves,
                  txsBroadcast,
                  broadcastErrors,
                  confirmed,
                  finalized,
                  totalItems,
                  elapsedMs,
                  txPerSec: sec > 0 ? finalized / sec : 0,
                  throughputBytesPerSec: sec > 0 ? finalizedBytes / sec : 0,
                })
              }
            })
            break
          }

          case "finalized": {
            const { finalizedBlockHashes, prunedBlockHashes } = event
            const lastHash =
              finalizedBlockHashes[finalizedBlockHashes.length - 1]
            if (!lastHash) break

            enqueue(async () => {
              const toUnpin = [...finalizedBlockHashes, ...prunedBlockHashes]
              follower.unpin(toUnpin).catch(() => {})

              if (!initialized || done) return

              // Read finalized nonces for all accounts
              const finNonces = await Promise.all(
                accounts.map((a) =>
                  readNonceAtBlock(monitorClient, a.signerHex, lastHash),
                ),
              )
              for (let i = 0; i < accounts.length; i++) {
                const a = accounts[i]!
                a.finalized = clamp(
                  finNonces[i]! - a.startNonce,
                  0,
                  a.items.length,
                )
              }

              if (onProgress) {
                const elapsedMs = Date.now() - startTime
                const sec = elapsedMs / 1000
                const finalized = accounts.reduce((s, a) => s + a.finalized, 0)
                const confirmed = accounts.reduce((s, a) => s + a.confirmed, 0)
                const totalItems = accounts.reduce(
                  (s, a) => s + a.items.length,
                  0,
                )
                const finalizedBytes = accounts.reduce(
                  (s, a) => s + (a.prefixBytes[a.finalized] ?? 0),
                  0,
                )
                onProgress({
                  waves: Math.max(...accounts.map((a) => a.waves)),
                  txsBroadcast: accounts.reduce(
                    (s, a) => s + a.txsBroadcast,
                    0,
                  ),
                  broadcastErrors: accounts.reduce(
                    (s, a) => s + a.broadcastErrors,
                    0,
                  ),
                  confirmed,
                  finalized,
                  totalItems,
                  elapsedMs,
                  txPerSec: sec > 0 ? finalized / sec : 0,
                  throughputBytesPerSec: sec > 0 ? finalizedBytes / sec : 0,
                })
              }

              if (allFinalized()) finish()
            })
            break
          }
        }
      },
      (error) => {
        if (!done) reject(error)
      },
    )

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
// Batch signing
// ---------------------------------------------------------------------------

async function signBatch(
  offlineStoreTx: OfflineStoreTx,
  signer: PolkadotSigner,
  items: Uint8Array[],
  fromIndex: number,
  toIndex: number,
  startNonce: number,
  blockNumber: number,
  blockHash: string,
): Promise<string[]> {
  const mortality = {
    mortal: true as const,
    period: 64,
    startAtBlock: { height: blockNumber, hash: blockHash },
  }
  const signed: string[] = []
  for (let i = fromIndex; i < toIndex; i++) {
    const offlineTx = offlineStoreTx({
      data: Binary.fromBytes(items[i] as Uint8Array),
    })
    signed.push(
      await offlineTx.sign(signer, { nonce: startNonce + i, mortality }),
    )
  }
  return signed
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

function hexToBytes(hex: string): Uint8Array {
  const h = hex.startsWith("0x") ? hex.slice(2) : hex
  const bytes = new Uint8Array(h.length / 2)
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(h.slice(i * 2, i * 2 + 2), 16)
  }
  return bytes
}

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

// ---------------------------------------------------------------------------
// Fast-path signer (bypasses per-tx metadata decode)
// ---------------------------------------------------------------------------

const SIGNER_TYPE_ID: Record<string, number> = {
  Ed25519: 0,
  Sr25519: 1,
  Ecdsa: 2,
}

/**
 * Create a PolkadotSigner that pre-decodes metadata once.
 *
 * PAPI's standard `getPolkadotSigner` calls `decAnyMetadata(metadata)` on
 * every `signTx()` invocation (~100ms each for typical chain metadata).
 * This wrapper decodes once and reuses the result, reducing per-tx overhead
 * to pure crypto (<5ms).
 */
async function createFastSigner(
  rawSign: (message: Uint8Array) => Promise<Uint8Array>,
  publicKey: Uint8Array,
  signingType: string,
  metadataRaw: Uint8Array,
): Promise<PolkadotSigner> {
  const [bindings, utils] = await Promise.all([
    import("@polkadot-api/substrate-bindings"),
    import("@polkadot-api/utils"),
  ])

  const decMeta = bindings.unifyMetadata(bindings.decAnyMetadata(metadataRaw))

  // Extract signed extension identifiers (order matters)
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const signedExts = (decMeta.extrinsic as any).signedExtensions
  const extList: Array<{ identifier: string }> = Array.isArray(signedExts)
    ? (signedExts[0] ?? [])
    : (Object.values(signedExts)[0] ?? [])
  const extIdentifiers: string[] = extList.map((e) => e.identifier)

  // Pre-compute address and signature assembly
  // For Polkadot/Substrate chains: MultiAddress::Id = [0x00, ...pubkey32]
  const addressBytes = new Uint8Array([0, ...publicKey])
  const sigTypeTag = SIGNER_TYPE_ID[signingType] ?? 1

  return {
    publicKey,
    signTx: async (
      callData: Uint8Array,
      signedExtensions: Record<
        string,
        { value: Uint8Array; additionalSigned: Uint8Array }
      >,
      _metadata: Uint8Array,
      _blockNumber?: number,
      hasher: (input: Uint8Array) => Uint8Array = bindings.Blake2256,
    ): Promise<Uint8Array> => {
      // Collect extra and additionalSigned from sign extensions
      const extra: Uint8Array[] = []
      const additionalSigned: Uint8Array[] = []
      for (const id of extIdentifiers) {
        const ext = signedExtensions[id]
        if (!ext) throw new Error(`Missing ${id} signed extension`)
        extra.push(ext.value)
        additionalSigned.push(ext.additionalSigned)
      }

      // Sign
      const toSign = utils.mergeUint8([callData, ...extra, ...additionalSigned])
      const signed = await rawSign(
        toSign.length > 256 ? hasher(toSign) : toSign,
      )

      // Assemble V4 signed extrinsic
      const preResult = utils.mergeUint8([
        bindings.extrinsicFormat.enc({ version: 4, type: "signed" }),
        addressBytes,
        new Uint8Array([sigTypeTag, ...signed]),
        ...extra,
        callData,
      ])
      return utils.mergeUint8([
        bindings.compact.enc(preResult.length),
        preResult,
      ])
    },
    // signBytes not used by the pipeline but required by the interface
    signBytes: async (data: Uint8Array) => rawSign(data),
  }
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
    inclusionLatency: null,
    finalizationLatency: null,
    inclusionLatenciesMs: [],
    finalizationLatenciesMs: [],
  }
}

/**
 * Compute summary statistics for a latency series.
 * Uses linear interpolation between adjacent ranks (same convention as
 * numpy.percentile / NIST). Returns null for an empty series.
 */
function computeLatencyStats(latenciesMs: number[]): LatencyStats | null {
  if (latenciesMs.length === 0) return null
  const sorted = [...latenciesMs].sort((a, b) => a - b)
  const sum = sorted.reduce((acc, v) => acc + v, 0)
  return {
    count: sorted.length,
    min: sorted[0] ?? 0,
    max: sorted[sorted.length - 1] ?? 0,
    mean: sum / sorted.length,
    p50: percentile(sorted, 50),
    p90: percentile(sorted, 90),
    p95: percentile(sorted, 95),
    p99: percentile(sorted, 99),
  }
}

function percentile(sorted: number[], p: number): number {
  const n = sorted.length
  if (n === 0) return 0
  if (n === 1) return sorted[0] ?? 0
  const rank = (p / 100) * (n - 1)
  const lo = Math.floor(rank)
  const hi = Math.ceil(rank)
  const frac = rank - lo
  return (sorted[lo] ?? 0) * (1 - frac) + (sorted[hi] ?? 0) * frac
}
