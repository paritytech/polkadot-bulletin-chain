// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Optimal bulk submission pipeline for Bulletin Chain.
 *
 * Event-driven algorithm that watches best and finalized blocks on one RPC
 * and broadcasts each signed transaction to every configured RPC.
 *
 * Key properties:
 * - Fresh signatures on every best block: each wave signs against the
 *   current best block's hash so the mortal era stays valid through reorgs
 *   and so re-broadcast doesn't reuse 30-minute-banned tx hashes
 * - Broadcasts to all RPC endpoints for maximum propagation
 * - Mortal transactions (64-block period) — long enough to absorb queueing
 *   delays under concurrent load while still letting stale waves expire
 * - Batch size computed from block weight/length limits
 * - Finalization-based completion — no false positives from pool nonces
 *
 * ## Lifecycle state machine
 *
 * ```
 *                    ┌──────────────────────────────┐
 *                    │   pipelineStore(api, signer, │
 *                    │     items, config) entry     │
 *                    └──────────────┬───────────────┘
 *                                   │
 *                                   ▼
 *                    ┌──────────────────────────────┐
 *                    │  openFollow() → chainHead    │
 *                    └──────────────┬───────────────┘
 *                                   │
 *                ┌──────────────────┼──────────────────┐
 *                │                  │                  │
 *                ▼                  ▼                  ▼
 *         "initialized"      "newBlock"        "bestBlockChanged"      "finalized"
 *         ┌──────────┐       ┌─────────┐       ┌──────────────┐        ┌────────────┐
 *         │ load     │       │ pre-    │       │ sign + wave  │        │ record     │
 *         │ metadata │       │ warm    │       │ broadcast    │        │ + emit     │
 *         │ + nonce  │       │ block-# │       │ + classify   │        │ Finalized  │
 *         └──────────┘       │ cache   │       └──────┬───────┘        └─────┬──────┘
 *                            └─────────┘              │                      │
 *                                                     │                      │
 *                              ╔══════════════════════╧══════════════════════╧═════╗
 *                              ║                Termination paths                  ║
 *                              ╚══════════════════════╤══════════════════════╤═════╝
 *                                                     │                      │
 *  ┌──────────────────────────────────────────────────┴──────┐               │
 *  │  failWithStall(reason) — converging error paths:        │               │
 *  │   • terminal RPC code (1010/1011/1015/1017–1020)        │               │
 *  │   • chainHead-event watchdog: no event for 18 s         │               │
 *  │   • no-progress watchdog: 20 best blocks without inclusion advance      │
 *  │   • chainHead error callback (non-StopError)            │               │
 *  └─────────────────────────────────┬───────────────────────┘               │
 *                                    │                                       │
 *                                    ▼                                       ▼
 *                              ┌──────────┐                            ┌──────────┐
 *                              │  reject  │                            │ finish() │
 *                              └──────────┘                            └──────────┘
 *                              (StoreStalled)                          ▲  ▲
 *                                                                      │  │
 *           finNonce ≥ expectedFinalNonce ───────────────────────────────  │
 *           completeOn:"best" + 2-block streak at target ──────────────────
 * ```
 *
 * `StopError` from the chainHead error callback is *not* a termination —
 * it triggers `openFollow()` to re-subscribe transparently while preserving
 * pipeline state (counters, cids, bootstrap).
 *
 * @packageDocumentation
 */

import type { JsonRpcProvider } from "@polkadot-api/json-rpc-provider"
import {
  createClient as createSubstrateClient,
  type FollowEventWithoutRuntime,
  type FollowResponse,
  StopError,
  type SubstrateClient,
} from "@polkadot-api/substrate-client"
import type { CID } from "multiformats/cid"
import {
  AccountId,
  Binary,
  getOfflineApi,
  type PolkadotSigner,
} from "polkadot-api"
import type { BulletinTypedApi } from "./async-client.js"
import {
  BulletinError,
  CidCodec,
  ErrorCode,
  HashAlgorithm,
  type UploadCallback,
  type UploadItem,
  UploadStatus,
} from "./types.js"
import { calculateCid, hashAlgorithmCodecToEnum } from "./utils.js"

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

import type { BlockLimits } from "./types.js"

export type { BlockLimits } from "./types.js"

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
  /** Lifecycle events (item-level). */
  onEvent?: UploadCallback
  /**
   * When the pipeline considers the upload done.
   *
   * - `"finalized"` (default): wait until the account nonce at a finalized
   *   block reaches `expectedFinalNonce`. Strongest guarantee.
   * - `"best"`: complete once `expectedFinalNonce` is observed at the best
   *   block for two consecutive `bestBlockChanged` events (single-block
   *   inclusions can be reorged away).
   */
  completeOn?: "best" | "finalized"
  /**
   * Optional mutable bootstrap cache. `pipelineStore` reads from it on entry
   * and writes to it on first init. Sharing the same object across retries
   * (in `uploadItemsImpl`) skips the `state_getMetadata` + offline-API
   * construction round-trip on every attempt.
   */
  bootstrap?: PipelineBootstrap
  /**
   * @internal — `uploadItemsImpl`-only. Pre-computed CIDs aligned with
   * `items[]` so retries skip recomputation. May be a `Promise<CID[]>` so
   * the caller can hand off computation in parallel with `pipelineStore`'s
   * bootstrap RPCs. Length is asserted to match `items.length` once
   * resolved.
   */
  precomputedCids?: CID[] | Promise<CID[]>
}

/** Mutable bootstrap cache, populated by `pipelineStore` on first call. */
export interface PipelineBootstrap {
  metadataRaw?: Uint8Array
  genesisHash?: string
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  offlineApi?: any
}

/** Final-state snapshot of pipeline progress (included in {@link PipelineResult}). */
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
  p99: number
}

/** Final result returned by {@link pipelineStore}. */
export interface PipelineResult extends PipelineStats {
  /** CIDs per item, computed from `(data, codec, hashAlgo)`. Matches input order. */
  cids: CID[]
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

/**
 * `cause` payload on a `BulletinError(STORE_STALLED)`: how many items the
 * chain had finalized when the store gave up. Callers can resume from the
 * chain's current state instead of re-submitting everything.
 */
export interface StallCause {
  readonly finalized: number
}

export function isStallError(
  err: unknown,
): err is BulletinError & { cause: StallCause } {
  return (
    err instanceof BulletinError &&
    err.code === ErrorCode.STORE_STALLED &&
    typeof (err.cause as StallCause | undefined)?.finalized === "number"
  )
}

function stallError(finalized: number, reason: string): BulletinError {
  return new BulletinError(
    `store stalled: ${reason}; finalized=${finalized}`,
    ErrorCode.STORE_STALLED,
    { finalized } satisfies StallCause,
  )
}

/**
 * Substrate transaction-pool RPC error codes. Stable across substrate
 * versions; see `client/rpc-api/src/author/error.rs`.
 */
enum AuthorRpcError {
  InvalidTransaction = 1010,
  UnknownValidity = 1011,
  TemporarilyBanned = 1012,
  AlreadyImported = 1013,
  TooLowPriority = 1014,
  CycleDetected = 1015,
  ImmediatelyDropped = 1016,
  InvalidTransactionV2 = 1017,
  UnauthorizedTransaction = 1018,
  UnknownCustomValidity = 1019,
  UnknownBuiltinValidity = 1020,
  FutureTransaction = 1021,
}

type RpcErrorClass = "terminal" | "retryable" | "already_imported" | "unknown"

function classifyAuthorRpcError(code: number | undefined): RpcErrorClass {
  switch (code) {
    // Bad sig / payment / unauthorized / cycle — no amount of retrying helps.
    case AuthorRpcError.InvalidTransaction:
    case AuthorRpcError.UnknownValidity:
    case AuthorRpcError.CycleDetected:
    case AuthorRpcError.InvalidTransactionV2:
    case AuthorRpcError.UnauthorizedTransaction:
    case AuthorRpcError.UnknownCustomValidity:
    case AuthorRpcError.UnknownBuiltinValidity:
      return "terminal"
    // Pool capacity / hash bans / future nonce — block production will drain
    // the pool and clear bans; keep going.
    case AuthorRpcError.TemporarilyBanned:
    case AuthorRpcError.TooLowPriority:
    case AuthorRpcError.ImmediatelyDropped:
    case AuthorRpcError.FutureTransaction:
      return "retryable"
    // The tx is sitting in the pool from a prior wave — count as success.
    case AuthorRpcError.AlreadyImported:
      return "already_imported"
    default:
      return "unknown"
  }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/** Minimal subset of the offline API surface used by pipelineStore. */
interface OfflineTxBuilder {
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
  ): Promise<Uint8Array>
}

interface OfflineTransactionStorage {
  store(args: { data: Uint8Array }): OfflineTxBuilder
  store_with_cid_config(args: {
    cid: { codec: bigint; hashing: { type: string } }
    data: Uint8Array
  }): OfflineTxBuilder
}

/** Returns true when the item should be sent via the lighter `store` extrinsic. */
/** @internal — exported for unit tests of the codec-dispatch decision. */
export function isDefaultCidConfig(item: UploadItem): boolean {
  return (
    (item.codec ?? CidCodec.Raw) === CidCodec.Raw &&
    (item.hashAlgo ?? HashAlgorithm.Blake2b256) === HashAlgorithm.Blake2b256
  )
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
 * 3. Sign each tx fresh, mortal era anchored to the current best block
 * 4. Broadcast every signed tx to every RPC endpoint
 *
 * Completion: when the account nonce at a finalized block ≥ `startNonce + items.length`.
 */
export async function pipelineStore(
  _api: BulletinTypedApi,
  signer: PolkadotSigner,
  items: UploadItem[],
  config: PipelineConfig,
): Promise<PipelineResult> {
  if (items.length === 0) return emptyResult()

  const { wsUrls, createProvider, blockLimits, onEvent } = config
  const completeOn = config.completeOn ?? "finalized"
  if (wsUrls.length === 0) {
    throw new Error("pipelineStore: at least one wsUrl is required")
  }

  // Kick off CID computation immediately so it runs in parallel with the
  // bootstrap RPCs (state_getMetadata, chain_getBlockHash, system_properties)
  // that the `initialized` handler awaits. We don't `await` here — the
  // upfront `emit(ItemStarted)` loop is deferred into `initialized` where
  // we can await this promise concurrently with the metadata fetch.
  const cidsPromise: Promise<CID[]> = (async () => {
    const supplied = config.precomputedCids
    if (supplied !== undefined) {
      const resolved = await Promise.resolve(supplied)
      if (resolved.length !== items.length) {
        throw new Error(
          `pipelineStore: precomputedCids length ${resolved.length} does not match items length ${items.length}`,
        )
      }
      return resolved
    }
    return Promise.all(
      items.map((item) =>
        calculateCid(
          item.data,
          item.codec ?? CidCodec.Raw,
          item.hashAlgo ?? HashAlgorithm.Blake2b256,
        ),
      ),
    )
  })()
  // `cids` is filled in the `initialized` handler — bind by reference.
  let cids: CID[] = []

  const totalItems = items.length
  let lastBestBlock: { hash: string; number: number } | undefined
  let lastFinalizedBlock: { hash: string; number: number } | undefined
  /**
   * Single point of emission. `block` is required for InBlock/Finalized and
   * looked up from the most recently observed best/finalized block; if the
   * pipeline hasn't seen one yet, the event is dropped silently rather than
   * emitting with a fabricated block — callers shouldn't see ItemInBlock
   * without a real block context.
   */
  const emit = (status: UploadStatus, i: number): void => {
    if (!onEvent) return
    const base = { index: i, total: totalItems, cid: cids[i]! }
    switch (status) {
      case UploadStatus.ItemStarted:
        onEvent({ type: status, ...base })
        return
      case UploadStatus.ItemInBlock: {
        if (!lastBestBlock) return
        onEvent({
          type: status,
          ...base,
          blockHash: lastBestBlock.hash,
          blockNumber: lastBestBlock.number,
        })
        return
      }
      case UploadStatus.ItemFinalized: {
        if (!lastFinalizedBlock) return
        onEvent({
          type: status,
          ...base,
          blockHash: lastFinalizedBlock.hash,
          blockNumber: lastFinalizedBlock.number,
        })
        return
      }
    }
  }
  // ItemInBlock is reorg-aware: emit once per (best-block, item) entry into
  // the chain. The flag is set when an item first appears at best and
  // cleared if a reorg drops it back. `tracking.maxConfirmedEver` lets us
  // skip the tail-clear loop entirely in the common no-reorg case.
  const inBlockEmitted: boolean[] = new Array(totalItems).fill(false)

  // Block-number cache populated lazily and proactively on `newBlock`.
  // Saves a `chain_getHeader` RPC on the bestBlockChanged / finalized hot
  // path when the hash has already been seen.
  const blockNumberByHash = new Map<string, number>()
  const MAX_BLOCK_NUMBER_CACHE = 64
  const getBlockNumber = async (hash: string): Promise<number> => {
    const hit = blockNumberByHash.get(hash)
    if (hit !== undefined) return hit
    const header = await monitorClient.request<{ number: string }>(
      "chain_getHeader",
      [hash],
    )
    const num = parseInt(header.number, 16)
    blockNumberByHash.set(hash, num)
    if (blockNumberByHash.size > MAX_BLOCK_NUMBER_CACHE) {
      // FIFO eviction (Map preserves insertion order).
      const firstKey = blockNumberByHash.keys().next().value
      if (firstKey !== undefined) blockNumberByHash.delete(firstKey)
    }
    return num
  }
  // ItemFinalized is monotonic (finalization is irreversible).
  let finalizedEmittedTo = 0

  // signerHex is used for SCALE state_call (the chain doesn't care about
  // SS58 prefix). signerSs58 is filled at `initialized` time from the chain's
  // `system_properties.ss58Format` — its only use is the
  // `system_accountNextIndex` RPC, which accepts any valid SS58 prefix anyway,
  // but reading the real value avoids assuming a network.
  const signerHex = Binary.toHex(signer.publicKey)
  let signerSs58 = ""

  // Pre-compute cumulative byte sizes for throughput reporting
  const prefixBytes = new Float64Array(items.length + 1)
  for (let i = 0; i < items.length; i++) {
    prefixBytes[i + 1] = (prefixBytes[i] ?? 0) + (items[i]?.data.length ?? 0)
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

  const startTime = Date.now()
  let startNonce = 0
  let expectedFinalNonce = 0
  let initialized = false
  let done = false
  let offlineStorage: OfflineTransactionStorage | undefined

  // Watchdog: a healthy WS+follow delivers a chainHead event every block
  // (~6 s). If we go too long without one, the underlying WS is most likely
  // dead and our chainHead restart can't recover (it would re-issue against
  // the same dead client). Bail out with a clear error so callers can retry
  // with a fresh client rather than silently looping forever.
  const STALL_TIMEOUT_MS = 18_000
  let lastChainEventAt = Date.now()
  let stallTimer: ReturnType<typeof setInterval> | undefined
  const touchChainEvent = (): void => {
    lastChainEventAt = Date.now()
  }

  // Substrate transaction-pool error codes from `author_submitExtrinsic`.
  // Codes are stable across substrate versions; see
  // client/rpc-api/src/author/error.rs for the source of truth.

  // No-progress watchdog: cap the time spent retrying on transient pool
  // pressure. If the account's confirmed nonce doesn't advance for this many
  // best-block events (≈ this many block times), bail with STORE_STALLED so
  // the outer retry layer can reconnect with fresh substrate clients.
  const MAX_NO_PROGRESS_BEST_BLOCKS = 20

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

  // Coalesce the per-wave "broadcasts deferred" diagnostic — only log when
  // the (code, count) changes, so a sustained pool-pressure window produces
  // 1-2 lines instead of one per block.
  let lastDeferredCode: number | undefined
  let lastDeferredCount = 0

  // Mutable scalars consumed by the per-block helpers (see WaveState).
  const tracking = {
    inclusionRecordedTo: 0,
    finalizationRecordedTo: 0,
    maxConfirmedEver: 0,
    bestAtTargetStreak: 0,
    prevConfirmed: 0,
    noProgressBestBlocks: 0,
  }
  const state: WaveState = {
    items,
    totalItems,
    cids,
    prefixBytes,
    counters,
    tracking,
    inclusionLatenciesMs,
    finalizationLatenciesMs,
    broadcastAtMs,
    inBlockEmitted,
    emit,
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
      while (queue.length > 0 && !done) {
        const fn = queue.shift()
        if (!fn) break
        try {
          await fn()
        } catch {
          /* swallow — surfaced via counters.broadcastErrors */
        }
      }
      draining = false
    }

    function teardown(): void {
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
      if (stallTimer !== undefined) {
        clearInterval(stallTimer)
        stallTimer = undefined
      }
    }

    function failWithStall(reason: string): void {
      if (done) return
      done = true
      teardown()
      reject(stallError(counters.finalized, reason))
    }

    function finish(): void {
      if (done) return
      done = true
      teardown()

      const durationMs = Date.now() - startTime
      const sec = durationMs / 1000
      const finalizedBytes = prefixBytes[counters.finalized] ?? 0
      resolve({
        cids,
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

    // ChainHead follow. `StopError` on the error callback restarts the
    // session transparently; only the first `initialized` event sets up state.
    let follower: FollowResponse
    const openFollow = (): FollowResponse =>
      monitorClient.chainHead(
        false,
        (event: FollowEventWithoutRuntime) => {
          if (done) return
          touchChainEvent()

          switch (event.type) {
            // initialized — read start nonce from finalized block
            case "initialized": {
              const hashes = event.finalizedBlockHashes
              const lastHash = hashes[hashes.length - 1]
              if (!lastHash) break
              // Restart path: state is already set up; just unpin and continue
              if (initialized) {
                follower.unpin(hashes).catch(() => {})
                break
              }
              enqueue(async () => {
                const cache = config.bootstrap
                const haveBootstrap = !!cache?.offlineApi && !!cache.metadataRaw
                // On retry, skip the heavy chain_getBlockHash + state_getMetadata
                // + getOfflineApi work; the caller passes the cached bootstrap.
                const [nonce, genesisHash, metadataRaw, properties] =
                  await Promise.all([
                    readNonceAtBlock(monitorClient, signerHex, lastHash),
                    cache?.genesisHash
                      ? Promise.resolve(cache.genesisHash)
                      : monitorClient.request<string>(
                          "chain_getBlockHash",
                          [0],
                        ),
                    cache?.metadataRaw
                      ? Promise.resolve(cache.metadataRaw)
                      : monitorClient
                          .request<string>("state_getMetadata", [])
                          .then((hex) => Binary.fromHex(hex)),
                    monitorClient
                      .request<{ ss58Format?: number | string }>(
                        "system_properties",
                        [],
                      )
                      .catch(() => ({}) as { ss58Format?: number | string }),
                  ])
                startNonce = nonce
                expectedFinalNonce = startNonce + items.length
                // `system_properties.ss58Format` is sometimes returned as a
                // string; coerce. If missing, fall back to 42 (generic Substrate).
                const ss58Format =
                  typeof properties.ss58Format === "string"
                    ? parseInt(properties.ss58Format, 10)
                    : (properties.ss58Format ?? 42)
                signerSs58 = AccountId(ss58Format).dec(signer.publicKey)

                const offlineApi = haveBootstrap
                  ? cache.offlineApi
                  : await (
                      getOfflineApi as (opts: {
                        genesis: string
                        getMetadata: () => Promise<Uint8Array>
                        // eslint-disable-next-line @typescript-eslint/no-explicit-any
                      }) => Promise<any>
                    )({
                      genesis: genesisHash,
                      getMetadata: async () => metadataRaw,
                    })
                if (cache) {
                  cache.metadataRaw = metadataRaw
                  cache.genesisHash = genesisHash
                  cache.offlineApi = offlineApi
                }
                offlineStorage = offlineApi.tx
                  .TransactionStorage as OfflineTransactionStorage

                // Now that bootstrap RPCs are done, await CIDs (which ran
                // concurrently). On the happy path this is already settled.
                cids = await cidsPromise
                state.cids = cids
                for (let i = 0; i < totalItems; i++) {
                  emit(UploadStatus.ItemStarted, i)
                }

                initialized = true
                follower.unpin(hashes).catch(() => {})
              })
              break
            }

            case "newBlock": {
              // Pre-warm the block-number cache so `bestBlockChanged` doesn't
              // pay the chain_getHeader round-trip on the critical path.
              // Pin is released in bulk on the next `finalized` event.
              const newHash = (event as { blockHash: string }).blockHash
              if (!blockNumberByHash.has(newHash)) {
                getBlockNumber(newHash).catch(() => {})
              }
              break
            }

            // bestBlockChanged — core submission loop
            case "bestBlockChanged": {
              const bestBlockHash = (
                event as { type: "bestBlockChanged"; bestBlockHash: string }
              ).bestBlockHash
              enqueue(async () => {
                if (!initialized || done) return

                const [bestNonce, bestBlockNumber] = await Promise.all([
                  monitorClient.request<number>("system_accountNextIndex", [
                    signerSs58,
                  ]),
                  getBlockNumber(bestBlockHash),
                ])
                lastBestBlock = { hash: bestBlockHash, number: bestBlockNumber }
                counters.confirmed = clamp(
                  bestNonce - startNonce,
                  0,
                  items.length,
                )

                recordInclusionLatency(state, Date.now())
                emitInBlockEvents(state)

                if (bestNonce >= expectedFinalNonce) {
                  // `completeOn: "best"` needs 2 consecutive best-block ticks
                  // at the target to avoid celebrating a one-block inclusion
                  // that may yet be reorged away.
                  tracking.bestAtTargetStreak += 1
                  if (
                    completeOn === "best" &&
                    tracking.bestAtTargetStreak >= 2
                  ) {
                    finish() // [TERMINATE-OK] completeOn:"best" 2-block streak
                  }
                  return
                }
                tracking.bestAtTargetStreak = 0

                const watchdog = runNoProgressWatchdog(
                  state,
                  MAX_NO_PROGRESS_BEST_BLOCKS,
                )
                if (watchdog.bail) {
                  failWithStall(watchdog.reason!) // [TERMINATE-STALL] no-progress watchdog
                  return
                }

                const fromIndex = Math.max(0, bestNonce - startNonce)
                const toIndex = computeBatchEnd(items, fromIndex, blockLimits)
                if (fromIndex >= toIndex) return

                // Re-sign every wave: reusing prior bytes is unsafe (stale
                // era → BadProof, banned-hash on pool eviction).
                // `initialized` guarantees `offlineStorage` is set.
                const signed = await signBatch({
                  storage: offlineStorage!,
                  signer,
                  items,
                  fromIndex,
                  toIndex,
                  startNonce,
                  anchor: { number: bestBlockNumber, hash: bestBlockHash },
                })

                // Record first-broadcast timestamp for each item in the batch
                const broadcastNow = Date.now()
                for (let i = fromIndex; i < toIndex; i++) {
                  if (broadcastAtMs[i] === undefined)
                    broadcastAtMs[i] = broadcastNow
                }

                const {
                  terminalCode: waveTerminalCode,
                  terminalMsg: waveTerminalMsg,
                  retryableCount: waveRetryableCount,
                  retryableLastCode: waveRetryableLastCode,
                } = await broadcastWave(signed, submitClients, counters)
                counters.waves++

                if (waveTerminalCode !== undefined) {
                  // biome-ignore lint/suspicious/noConsole: terminal — show the user why
                  console.warn(
                    `[pipelineStore] wave #${counters.waves}: terminal RPC error (code=${waveTerminalCode}, msg=${waveTerminalMsg?.slice(0, 120)})`,
                  )
                  // [TERMINATE-STALL] terminal RPC code seen in broadcastWave
                  failWithStall(
                    `terminal RPC error (code=${waveTerminalCode}): ${waveTerminalMsg}`,
                  )
                  return
                }
                if (waveRetryableCount > 0) {
                  // Diagnostic: only warn when the wave's signature changes
                  // (different code or different deferred count) so a
                  // sustained 20-block pool-pressure window produces 1–2
                  // log lines instead of 20.
                  if (
                    lastDeferredCode !== waveRetryableLastCode ||
                    lastDeferredCount !== waveRetryableCount
                  ) {
                    // biome-ignore lint/suspicious/noConsole: progress signal under pool pressure
                    console.warn(
                      `[pipelineStore] wave #${counters.waves}: ${waveRetryableCount} broadcasts deferred (last code=${waveRetryableLastCode}); no-progress ${tracking.noProgressBestBlocks}/${MAX_NO_PROGRESS_BEST_BLOCKS}`,
                    )
                    lastDeferredCode = waveRetryableLastCode
                    lastDeferredCount = waveRetryableCount
                  }
                } else {
                  // A clean wave resets the dedup state so the next stutter
                  // produces a fresh log line.
                  lastDeferredCode = undefined
                  lastDeferredCount = 0
                }
              })
              break
            }

            // finalized — check completion, unpin blocks
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

                const [finNonce, finBlockNumber] = await Promise.all([
                  readNonceAtBlock(monitorClient, signerHex, lastHash),
                  getBlockNumber(lastHash),
                ])
                lastFinalizedBlock = { hash: lastHash, number: finBlockNumber }
                counters.finalized = clamp(
                  finNonce - startNonce,
                  0,
                  items.length,
                )

                recordFinalizationLatency(state, Date.now())

                // Emit ItemFinalized once per item (monotonic).
                while (finalizedEmittedTo < counters.finalized) {
                  emit(UploadStatus.ItemFinalized, finalizedEmittedTo)
                  finalizedEmittedTo++
                }

                if (finNonce >= expectedFinalNonce) {
                  finish() // [TERMINATE-OK] finalized nonce reached target
                }
              })
              break
            }
          }
        },
        (error) => {
          if (done) return
          // `StopError` = JSON-RPC v2 chainHead `stop`. The session is torn
          // down internally; open a new one and resume.
          if (error instanceof StopError) {
            follower = openFollow()
            return
          }
          // [TERMINATE-STALL] chainHead error callback (non-Stop)
          done = true
          teardown()
          reject(error)
        },
      )

    follower = openFollow()

    stallTimer = setInterval(() => {
      if (done) return
      if (Date.now() - lastChainEventAt > STALL_TIMEOUT_MS) {
        // [TERMINATE-STALL] chainHead-event watchdog
        failWithStall(`no chainHead event for ${STALL_TIMEOUT_MS} ms`)
      }
    }, 6_000)
    // Don't pin the Node event loop on the watchdog alone; the chainHead
    // subscription keeps the loop alive while the pipeline is active.
    stallTimer.unref?.()
  })
}

// ---------------------------------------------------------------------------
// Per-best-block helpers
// ---------------------------------------------------------------------------

/**
 * Mutable state shared across the per-block helpers. Scalars that change
 * inside helpers are grouped under `tracking` so they pass by reference.
 *
 *       new bestBlock ──► recordInclusionLatency ──► emitInBlockEvents
 *                                                     │
 *                                                     ▼
 *                       runNoProgressWatchdog ◄──── update prevConfirmed
 *                              │
 *                              ▼ (bail?)
 *                          failWithStall
 */
interface WaveState {
  items: UploadItem[]
  totalItems: number
  cids: CID[]
  prefixBytes: Float64Array
  counters: {
    waves: number
    txsBroadcast: number
    broadcastErrors: number
    confirmed: number
    finalized: number
  }
  tracking: {
    inclusionRecordedTo: number
    finalizationRecordedTo: number
    maxConfirmedEver: number
    bestAtTargetStreak: number
    prevConfirmed: number
    noProgressBestBlocks: number
  }
  inclusionLatenciesMs: number[]
  finalizationLatenciesMs: number[]
  broadcastAtMs: Array<number | undefined>
  inBlockEmitted: boolean[]
  emit: (status: UploadStatus, i: number) => void
}

/** Push inclusion-latency entries for items newly observed at best block. */
function recordInclusionLatency(s: WaveState, observedAt: number): void {
  if (s.counters.confirmed <= s.tracking.inclusionRecordedTo) return
  for (let i = s.tracking.inclusionRecordedTo; i < s.counters.confirmed; i++) {
    const broadcast = s.broadcastAtMs[i]
    if (broadcast !== undefined) {
      s.inclusionLatenciesMs.push(observedAt - broadcast)
    }
  }
  s.tracking.inclusionRecordedTo = s.counters.confirmed
}

/** Push finalization-latency entries for items newly observed at finality. */
function recordFinalizationLatency(s: WaveState, observedAt: number): void {
  if (s.counters.finalized <= s.tracking.finalizationRecordedTo) return
  for (
    let i = s.tracking.finalizationRecordedTo;
    i < s.counters.finalized;
    i++
  ) {
    const broadcast = s.broadcastAtMs[i]
    if (broadcast !== undefined) {
      s.finalizationLatenciesMs.push(observedAt - broadcast)
    }
  }
  s.tracking.finalizationRecordedTo = s.counters.finalized
}

/**
 * Reorg-aware ItemInBlock emission. Items currently in best chain that
 * haven't been emitted fire now; items that dropped out (reorg from a
 * prior high) have their emitted-flag cleared so they refire when
 * re-included. No-reorg case = no tail walk.
 */
function emitInBlockEvents(s: WaveState): void {
  for (let i = 0; i < s.counters.confirmed; i++) {
    if (!s.inBlockEmitted[i]) {
      s.emit(UploadStatus.ItemInBlock, i)
      s.inBlockEmitted[i] = true
    }
  }
  if (s.counters.confirmed < s.tracking.maxConfirmedEver) {
    for (let i = s.counters.confirmed; i < s.tracking.maxConfirmedEver; i++) {
      s.inBlockEmitted[i] = false
    }
  }
  if (s.counters.confirmed > s.tracking.maxConfirmedEver) {
    s.tracking.maxConfirmedEver = s.counters.confirmed
  }
}

/**
 * No-progress watchdog: pool pressure for a few blocks is fine, but if no
 * inclusion progress is made for `max` consecutive best blocks, signal bail.
 */
function runNoProgressWatchdog(
  s: WaveState,
  max: number,
): { bail: boolean; reason?: string } {
  if (s.counters.confirmed > s.tracking.prevConfirmed) {
    s.tracking.noProgressBestBlocks = 0
  } else {
    s.tracking.noProgressBestBlocks += 1
    if (s.tracking.noProgressBestBlocks >= max) {
      return {
        bail: true,
        reason: `no inclusion progress for ${max} best blocks (likely persistent pool pressure)`,
      }
    }
  }
  s.tracking.prevConfirmed = s.counters.confirmed
  return { bail: false }
}

// ---------------------------------------------------------------------------
// Batch signing
// ---------------------------------------------------------------------------

interface WaveResult {
  terminalCode?: number
  terminalMsg?: string
  retryableCount: number
  retryableLastCode?: number
}

/**
 * Broadcast every signed tx to every RPC, classifying each rejection by
 * code. Returns the wave summary the caller uses to drive bail/diagnostic
 * decisions. Mutates `counters.txsBroadcast` / `counters.broadcastErrors`.
 */
async function broadcastWave(
  signed: string[],
  submitClients: SubstrateClient[],
  counters: { txsBroadcast: number; broadcastErrors: number },
): Promise<WaveResult> {
  let terminalCode: number | undefined
  let terminalMsg: string | undefined
  let retryableCount = 0
  let retryableLastCode: number | undefined
  const promises: Promise<void>[] = []
  for (const hex of signed) {
    for (const client of submitClients) {
      promises.push(
        client
          .request("author_submitExtrinsic", [hex])
          .then(() => {
            counters.txsBroadcast++
          })
          .catch((err: unknown) => {
            const e = err as { code?: number; message?: string }
            switch (classifyAuthorRpcError(e?.code)) {
              case "already_imported":
                counters.txsBroadcast++
                return
              case "terminal":
                counters.broadcastErrors++
                if (terminalCode === undefined) {
                  terminalCode = e.code
                  terminalMsg = e.message
                }
                return
              default:
                // retryable + unknown both fall here.
                counters.broadcastErrors++
                retryableCount++
                retryableLastCode = e?.code
            }
          }),
      )
    }
  }
  await Promise.allSettled(promises)
  return { terminalCode, terminalMsg, retryableCount, retryableLastCode }
}

interface SignBatchArgs {
  storage: OfflineTransactionStorage
  signer: PolkadotSigner
  items: UploadItem[]
  fromIndex: number
  toIndex: number
  startNonce: number
  /** Mortal era anchor — the current best block. */
  anchor: { number: number; hash: string }
}

/**
 * Sign every item in `[fromIndex, toIndex)`. Mortal era period is 64 blocks
 * (vs. the spec's recommended 4) so a tx still validates if this handler
 * lags several blocks behind the captured anchor under concurrent load.
 */
async function signBatch(args: SignBatchArgs): Promise<string[]> {
  const { storage, signer, items, fromIndex, toIndex, startNonce, anchor } =
    args
  const mortality = {
    mortal: true as const,
    period: 64,
    startAtBlock: { height: anchor.number, hash: anchor.hash },
  }
  const signed: string[] = new Array(toIndex - fromIndex)
  for (let i = fromIndex; i < toIndex; i++) {
    const item = items[i] as UploadItem
    const tx = isDefaultCidConfig(item)
      ? storage.store({ data: item.data })
      : storage.store_with_cid_config({
          cid: {
            codec: BigInt(item.codec ?? CidCodec.Raw),
            hashing: hashAlgorithmCodecToEnum(
              item.hashAlgo ?? HashAlgorithm.Blake2b256,
            ),
          },
          data: item.data,
        })
    const bytes = await tx.sign(signer, { nonce: startNonce + i, mortality })
    signed[i - fromIndex] = Binary.toHex(bytes)
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
  items: UploadItem[],
  fromIndex: number,
  limits: BlockLimits,
): number {
  let toIndex = fromIndex
  let accWeight = 0n
  let accLength = 0

  while (toIndex < items.length) {
    const size = items[toIndex]?.data.length ?? 0
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

function decodeU32LE(hex: string): number {
  return Buffer.from(
    hex.startsWith("0x") ? hex.slice(2) : hex,
    "hex",
  ).readUInt32LE(0)
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value))
}

function emptyResult(): PipelineResult {
  return {
    cids: [],
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
