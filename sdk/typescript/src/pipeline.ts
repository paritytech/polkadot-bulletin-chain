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
  api: BulletinTypedApi,
  signer: PolkadotSigner | undefined,
  items: UploadItem[],
  config: PipelineConfig,
): Promise<PipelineResult> {
  if (items.length === 0) return emptyResult()

  // Unsigned mode: caller passes `undefined` for signer. Items are
  // submitted as bare extrinsics (preimage-authorized on chain) via the
  // same shared chainHead subscription + TBCH reconciler as signed.
  // Bypasses signing, nonce tracking, hijack detection, and the retry
  // queue — each unsigned tx is independent and identified by its
  // content hash.
  const unsigned = signer === undefined

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
  // Per-item TBCH state — see WaveState docs. The reconciler populates
  // `storedAt[i]` when the chain confirms item i's content was stored at
  // or after `submissionAnchorBlock[i]`.
  const submissionAnchorBlock: Array<number | undefined> = new Array(
    totalItems,
  ).fill(undefined)
  const storedAt: Array<
    { blockNumber: number; extrinsicIndex: number } | undefined
  > = new Array(totalItems).fill(undefined)
  // blake2b-256 hex (no 0x) per item — keys for TBCH lookups. Filled in
  // the `initialized` handler from CIDs (avoids double-hashing the data
  // when the CID itself uses blake2b-256, which is the default).
  let contentHashesHex: string[] = []
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
        // Prefer the block where the item *actually* landed (from TBCH)
        // over the current best block — important when items confirm in a
        // block earlier than the one we're reconciling against.
        const at = storedAt[i] ?? {
          blockNumber: lastBestBlock.number,
          extrinsicIndex: undefined as number | undefined,
        }
        onEvent({
          type: status,
          ...base,
          blockHash: lastBestBlock.hash,
          blockNumber: at.blockNumber,
          extrinsicIndex: storedAt[i]?.extrinsicIndex,
        })
        return
      }
      case UploadStatus.ItemFinalized: {
        if (!lastFinalizedBlock) return
        onEvent({
          type: status,
          ...base,
          blockHash: lastFinalizedBlock.hash,
          blockNumber: storedAt[i]?.blockNumber ?? lastFinalizedBlock.number,
          extrinsicIndex: storedAt[i]?.extrinsicIndex,
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
  // signerHex is used for SCALE state_call (the chain doesn't care about
  // SS58 prefix). signerSs58 is filled at `initialized` time from the chain's
  // `system_properties.ss58Format` — its only use is the
  // `system_accountNextIndex` RPC, which accepts any valid SS58 prefix anyway,
  // but reading the real value avoids assuming a network.
  // Both empty in unsigned mode (no signer, no nonce tracking).
  const signerHex = signer ? Binary.toHex(signer.publicKey) : ""
  let signerSs58 = ""

  // Per-item broadcast tracking — used only in unsigned mode where there
  // are no nonces to dedupe against. Set after first successful broadcast;
  // wave dispatcher skips items already broadcast to avoid spamming the pool.
  const broadcastedItems = new Set<number>()

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

  // Per-item assigned nonce. Initialized to `startNonce + i` for each item;
  // can change when an item's nonce gets hijacked and the retry queue
  // re-assigns a fresh nonce.
  const itemNonce: Array<number | undefined> = new Array(totalItems).fill(
    undefined,
  )
  // Retry budget per item — incremented each time we re-sign with a fresh
  // nonce. Caps at MAX_RETRY_ATTEMPTS; beyond that we emit ItemFailed.
  const retryAttempts: number[] = new Array(totalItems).fill(0)
  // Highest nonce ever assigned to any item. Retries pull `nextFreeNonce++`
  // to avoid colliding with already-broadcast nonces.
  let nextFreeNonce = 0
  // Items whose retry budget was exhausted — these get `ItemFailed` once
  // and are excluded from subsequent waves.
  const failedItems = new Set<number>()
  const MAX_RETRY_ATTEMPTS = 3

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
  const finalizedEmitted: boolean[] = new Array(totalItems).fill(false)
  const state: WaveState = {
    items,
    totalItems,
    cids,
    prefixBytes,
    contentHashesHex,
    submissionAnchorBlock,
    storedAt,
    counters,
    tracking,
    inclusionLatenciesMs,
    finalizationLatenciesMs,
    broadcastAtMs,
    inBlockEmitted,
    finalizedEmitted,
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
              // Restart path: state is already set up, but chainHead
              // events that fired during the disconnect were lost. Run a
              // one-shot TBCH reconcile at the new finalized tip to pick
              // up any items that landed during the gap and emit their
              // ItemInBlock / ItemFinalized events. This relies entirely
              // on current chain state — works even if event blocks were
              // pruned during a long disconnect.
              if (initialized) {
                follower.unpin(hashes).catch(() => {})
                enqueue(async () => {
                  try {
                    const newTipNumber = await getBlockNumber(lastHash)
                    // Lagging-RPC guard: if we failed over to a node that
                    // hasn't caught up to where we were before the drop,
                    // skip recovery — the live observers will run as the
                    // node catches up and natural reconciliation kicks in.
                    if (
                      lastFinalizedBlock !== undefined &&
                      newTipNumber < lastFinalizedBlock.number
                    ) {
                      // biome-ignore lint/suspicious/noConsole: ops visibility
                      console.warn(
                        `[pipelineStore] reconnect: new tip #${newTipNumber} is behind last seen finalized #${lastFinalizedBlock.number}; waiting for catch-up`,
                      )
                      return
                    }
                    lastFinalizedBlock = {
                      hash: lastHash,
                      number: newTipNumber,
                    }
                    // Set lastBestBlock too so InBlock emissions during
                    // recovery carry a consistent block reference (the
                    // pre-disconnect lastBestBlock could be staler than
                    // newTip after a long gap).
                    if (
                      !lastBestBlock ||
                      lastBestBlock.number < newTipNumber
                    ) {
                      lastBestBlock = { hash: lastHash, number: newTipNumber }
                    }
                    await reconcileAtBlock(api, state, lastHash)
                    recordInclusionLatency(state, Date.now())
                    recordFinalizationLatency(state, Date.now())
                    // Newly-confirmed items emit ItemInBlock + ItemFinalized
                    // so callers see the standard Started → InBlock →
                    // Finalized progression even when the in-block event
                    // happened during the disconnect window.
                    emitInBlockEvents(state)
                    emitFinalizedEvents(state)
                  } catch (err) {
                    // biome-ignore lint/suspicious/noConsole: ops visibility
                    console.warn(
                      `[pipelineStore] reconnect reconcile failed: ${err instanceof Error ? err.message : String(err)}; live observers will take over`,
                    )
                  }
                })
                break
              }
              enqueue(async () => {
                const cache = config.bootstrap
                const haveBootstrap =
                  !unsigned && !!cache?.offlineApi && !!cache.metadataRaw
                // On retry, skip the heavy chain_getBlockHash + state_getMetadata
                // + getOfflineApi work; the caller passes the cached bootstrap.
                // Unsigned mode skips ALL signed-only bootstrap (nonce, ss58,
                // genesis, metadata, offlineApi) — it just needs the online
                // `api.tx` to build bareTxs.
                const reads = unsigned
                  ? await Promise.all([
                      Promise.resolve(0), // nonce — unused
                      Promise.resolve(""), // genesisHash — unused
                      Promise.resolve(new Uint8Array(0)), // metadataRaw — unused
                      Promise.resolve(
                        {} as { ss58Format?: number | string },
                      ),
                    ])
                  : await Promise.all([
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
                const [nonce, genesisHash, metadataRaw, properties] = reads
                if (!unsigned) {
                  startNonce = nonce
                  expectedFinalNonce = startNonce + items.length
                  // Each item gets its sequential nonce; nextFreeNonce
                  // points at the first unused slot, used for hijack retries.
                  for (let i = 0; i < totalItems; i++) {
                    itemNonce[i] = startNonce + i
                  }
                  nextFreeNonce = startNonce + totalItems
                  // `system_properties.ss58Format` is sometimes returned as a
                  // string; coerce. If missing, fall back to 42 (generic Substrate).
                  const ss58Format =
                    typeof properties.ss58Format === "string"
                      ? parseInt(properties.ss58Format, 10)
                      : (properties.ss58Format ?? 42)
                  signerSs58 = AccountId(ss58Format).dec(signer!.publicKey)

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
                }

                // Now that bootstrap RPCs are done, await CIDs (which ran
                // concurrently). On the happy path this is already settled.
                cids = await cidsPromise
                state.cids = cids
                // The pallet keys `TransactionByContentHash` by the item's
                // content hash, which is `H(data)` under the user-chosen
                // hash algo (Blake2b-256 by default, also SHA2-256 or
                // Keccak-256 if the user picked a different CID config).
                // That value is exactly the CID's multihash digest, so we
                // never need to re-hash the data — works uniformly for all
                // 3 supported algos. PAPI expects H256 args as `0x`-hex.
                contentHashesHex = cids.map((cid) =>
                  Binary.toHex(cid.multihash.digest),
                )
                state.contentHashesHex = contentHashesHex
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

                // Signed mode reads two nonces (poolNonce for dispatch
                // dedup + chainNonce for hijack detection). Unsigned skips
                // both — each unsigned tx is independent, no nonces.
                const [poolNonce, chainNonce, bestBlockNumber] = unsigned
                  ? await Promise.all([
                      Promise.resolve(0),
                      Promise.resolve(0),
                      getBlockNumber(bestBlockHash),
                    ])
                  : await Promise.all([
                      monitorClient.request<number>(
                        "system_accountNextIndex",
                        [signerSs58],
                      ),
                      readNonceAtBlock(
                        monitorClient,
                        signerHex,
                        bestBlockHash,
                      ),
                      getBlockNumber(bestBlockHash),
                    ])
                lastBestBlock = { hash: bestBlockHash, number: bestBlockNumber }

                // Reconcile via TBCH state at this best block — populates
                // `storedAt[i]` for items confirmed at or before B. Then
                // `emitInBlockEvents` updates `counters.confirmed` and fires
                // `ItemInBlock` with the correct `extrinsicIndex`.
                await reconcileAtBlock(api, state, bestBlockHash)
                recordInclusionLatency(state, Date.now())
                emitInBlockEvents(state)

                // Detect hijack: pending items whose nonce was used by the
                // chain (chainNonce > itemNonce) but whose content_hash
                // isn't in TBCH → someone else's tx took our nonce slot.
                // Uses on-chain `chainNonce` only — `poolNonce` would
                // false-positive on our own in-pool txs. Skip entirely in
                // unsigned mode (no nonces to track).
                if (!unsigned) {
                  for (let i = 0; i < totalItems; i++) {
                    if (storedAt[i] !== undefined) continue
                    if (failedItems.has(i)) continue
                    const nonce = itemNonce[i]
                    if (nonce === undefined) continue
                    if (chainNonce <= nonce) continue
                    // Hijacked at nonce `nonce`.
                    const attempts = retryAttempts[i] ?? 0
                    if (attempts >= MAX_RETRY_ATTEMPTS) {
                      // biome-ignore lint/suspicious/noConsole: visibility for permanent failure
                      console.warn(
                        `[pipelineStore] item ${i}: hijacked ${MAX_RETRY_ATTEMPTS} times (nonce ${nonce}); giving up`,
                      )
                      failedItems.add(i)
                      onEvent?.({
                        type: UploadStatus.ItemFailed,
                        index: i,
                        total: totalItems,
                        cid: cids[i] as CID,
                        error: new BulletinError(
                          `item ${i}: nonce slot repeatedly hijacked by another transaction from the same signer (${MAX_RETRY_ATTEMPTS} attempts)`,
                          ErrorCode.HIJACK_BUDGET_EXCEEDED,
                        ),
                      })
                      continue
                    }
                    // Pick the next free nonce. Use max(local counter,
                    // poolNonce) so concurrent same-account uploaders pick
                    // non-overlapping slots — the pool's view of "next
                    // available" already accounts for OTHER processes'
                    // pending txs, which our local counter doesn't see.
                    const fresh = Math.max(nextFreeNonce, poolNonce)
                    nextFreeNonce = fresh + 1
                    // biome-ignore lint/suspicious/noConsole: ops visibility for transient hijacks
                    console.warn(
                      `[pipelineStore] item ${i}: nonce ${nonce} hijacked at chain=${chainNonce}; retry attempt ${attempts + 1}/${MAX_RETRY_ATTEMPTS} with nonce ${fresh}`,
                    )
                    itemNonce[i] = fresh
                    retryAttempts[i] = attempts + 1
                  }
                  // Rolling expected-final-nonce keeps the "all-done" check
                  // aware of fresh nonces consumed by retries.
                  expectedFinalNonce = nextFreeNonce
                }

                // Two pending sets:
                //   pendingForFinal — anything not yet stored and not
                //     failed; drives the "are we done" check.
                //   pendingForBroadcast — subset of pendingForFinal whose
                //     itemNonce is NOT already in the pool. Items in pool
                //     skipped to avoid spamming with new-era variants.
                //     Hijack-reassigned items have itemNonce ≥ poolNonce.
                const pendingForFinal: number[] = []
                const pendingForBroadcast: number[] = []
                for (let i = 0; i < totalItems; i++) {
                  if (storedAt[i] !== undefined) continue
                  if (failedItems.has(i)) continue
                  pendingForFinal.push(i)
                  // Dispatch dedup: signed uses nonce-vs-pool check;
                  // unsigned tracks per-item broadcast state locally
                  // (no nonces). Either way: skip if already in pool.
                  if (unsigned) {
                    if (broadcastedItems.has(i)) continue
                  } else {
                    const nonce = itemNonce[i]
                    if (nonce !== undefined && nonce < poolNonce) continue
                  }
                  pendingForBroadcast.push(i)
                }
                if (pendingForFinal.length === 0) {
                  // Everyone stored or failed. The `finalized` handler
                  // will issue the termination call.
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

                if (pendingForBroadcast.length === 0) return
                const waveIndexes = selectWaveBatch(
                  items,
                  pendingForBroadcast,
                  blockLimits,
                )
                if (waveIndexes.length === 0) return

                // Re-sign every wave: reusing prior bytes is unsafe (stale
                // era → BadProof, banned-hash on pool eviction). The wave
                // also picks up any items whose itemNonce was bumped above
                // for hijack recovery.
                const signed = await signBatch({
                  storage: offlineStorage!,
                  api,
                  signer,
                  items,
                  indexes: waveIndexes,
                  itemNonce,
                  anchor: { number: bestBlockNumber, hash: bestBlockHash },
                })

                // Record first-broadcast timestamp + wave-specific anchor
                // block for each item in this batch. The anchor is used by
                // the TBCH reconciler to distinguish "TBCH refreshed by our
                // wave" from "pre-existing entry from an earlier upload."
                const broadcastNow = Date.now()
                for (const i of waveIndexes) {
                  if (broadcastAtMs[i] === undefined)
                    broadcastAtMs[i] = broadcastNow
                  submissionAnchorBlock[i] = bestBlockNumber
                  if (unsigned) broadcastedItems.add(i)
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

                // Unsigned mode skips finNonce — completion is purely
                // TBCH-driven (all items have storedAt set).
                const [finNonce, finBlockNumber] = unsigned
                  ? await Promise.all([
                      Promise.resolve(0),
                      getBlockNumber(lastHash),
                    ])
                  : await Promise.all([
                      readNonceAtBlock(monitorClient, signerHex, lastHash),
                      getBlockNumber(lastHash),
                    ])
                lastFinalizedBlock = { hash: lastHash, number: finBlockNumber }

                // Reconcile at the finalized block — populates `storedAt[i]`
                // for any item whose TBCH entry exists at finalization. Then
                // emit ItemFinalized (monotonic) with `extrinsicIndex`.
                await reconcileAtBlock(api, state, lastHash)
                recordFinalizationLatency(state, Date.now())
                emitFinalizedEvents(state)

                if (
                  counters.finalized >= items.length ||
                  (!unsigned && finNonce >= expectedFinalNonce)
                ) {
                  finish() // [TERMINATE-OK] all items finalized OR nonce reached target
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
  /** blake2b-256 hex (no 0x) per item — keys for TBCH reads. */
  contentHashesHex: string[]
  /**
   * Best-block number at sign time per item, per *current* wave attempt.
   * The reconciler uses this to distinguish "TBCH entry from our wave" vs
   * "pre-existing entry from a prior upload of the same content." Updated
   * each wave when an item is re-signed with a fresh era.
   */
  submissionAnchorBlock: Array<number | undefined>
  /**
   * Where on chain each item landed, populated by the TBCH reconciler.
   * Set when `TBCH[contentHash]` exists and its block ≥ submissionAnchorBlock.
   * `extrinsicIndex` is surfaced on `ItemInBlock` / `ItemFinalized` events.
   */
  storedAt: Array<{ blockNumber: number; extrinsicIndex: number } | undefined>
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
  finalizedEmitted: boolean[]
  emit: (status: UploadStatus, i: number) => void
}

/**
 * Per-block TBCH reconciliation. Reads `TransactionByContentHash` for
 * every item whose `storedAt` isn't set yet, in one batched RPC. For each
 * entry whose block-number is at or after that item's `submissionAnchorBlock`,
 * marks the item as stored.
 *
 * The temporal predicate (`tbch.blockNumber ≥ submissionAnchorBlock[i]`)
 * is what distinguishes "our wave landed" from "this content already
 * existed on chain from a previous upload." Pre-existing entries are
 * ignored until the chain refreshes them past our anchor — which happens
 * when our `store` extrinsic executes successfully.
 */
async function reconcileAtBlock(
  api: BulletinTypedApi,
  state: WaveState,
  blockHash: string,
): Promise<void> {
  const pending: number[] = []
  for (let i = 0; i < state.totalItems; i++) {
    if (
      state.storedAt[i] === undefined &&
      state.submissionAnchorBlock[i] !== undefined
    ) {
      pending.push(i)
    }
  }
  if (pending.length === 0) return
  const hashes = pending.map((i) => state.contentHashesHex[i] as string)
  const tbch = await readStoredAtBlockBatch(api, hashes, blockHash)
  for (let k = 0; k < pending.length; k++) {
    const i = pending[k]!
    const entry = tbch.get(normalizeHex(hashes[k] as string))
    if (!entry) continue
    const anchor = state.submissionAnchorBlock[i]!
    if (entry.blockNumber >= anchor) {
      state.storedAt[i] = entry
    }
  }
}

/** Push inclusion-latency entries for items newly observed at best block. */
function recordInclusionLatency(s: WaveState, observedAt: number): void {
  for (let i = 0; i < s.totalItems; i++) {
    if (s.storedAt[i] !== undefined && !s.inBlockEmitted[i]) {
      const broadcast = s.broadcastAtMs[i]
      if (broadcast !== undefined) {
        s.inclusionLatenciesMs.push(observedAt - broadcast)
      }
    }
  }
}

/** Push finalization-latency entries for items newly observed at finality. */
function recordFinalizationLatency(s: WaveState, observedAt: number): void {
  for (let i = 0; i < s.totalItems; i++) {
    if (s.storedAt[i] !== undefined && !s.finalizedEmitted[i]) {
      const broadcast = s.broadcastAtMs[i]
      if (broadcast !== undefined) {
        s.finalizationLatenciesMs.push(observedAt - broadcast)
      }
    }
  }
}

/**
 * Reorg-aware ItemInBlock emission. Items now in best chain (storedAt
 * set) that haven't been emitted fire now; items whose storedAt cleared
 * (reorg out of the canonical chain) have their emitted-flag reset so
 * they refire on re-inclusion. With per-item TBCH tracking, items may
 * confirm in non-consecutive order under unusual conditions — this loop
 * handles either case uniformly.
 */
function emitInBlockEvents(s: WaveState): void {
  let confirmed = 0
  for (let i = 0; i < s.totalItems; i++) {
    if (s.storedAt[i] !== undefined) {
      confirmed++
      if (!s.inBlockEmitted[i]) {
        s.emit(UploadStatus.ItemInBlock, i)
        s.inBlockEmitted[i] = true
      }
    } else if (s.inBlockEmitted[i]) {
      // Reorged out — clear flag so the next confirmation re-emits.
      s.inBlockEmitted[i] = false
    }
  }
  s.counters.confirmed = confirmed
  if (confirmed > s.tracking.maxConfirmedEver) {
    s.tracking.maxConfirmedEver = confirmed
  }
}

/**
 * Monotonic ItemFinalized emission. An item emits once when its TBCH
 * entry is observed at a finalized block hash (finalization is
 * irreversible, so this flag never clears).
 */
function emitFinalizedEvents(s: WaveState): void {
  let finalized = 0
  for (let i = 0; i < s.totalItems; i++) {
    if (s.storedAt[i] !== undefined) {
      finalized++
      if (!s.finalizedEmitted[i]) {
        s.emit(UploadStatus.ItemFinalized, i)
        s.finalizedEmitted[i] = true
      }
    }
  }
  s.counters.finalized = finalized
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
  /**
   * Offline storage for signed mode. Pre-built offline-API offers faster
   * signing than the online api's `tx.sign(…)`. Ignored when `signer`
   * is `undefined` — unsigned txs build bareTx via the online `api`.
   */
  storage: OfflineTransactionStorage
  /** Online api — used by unsigned mode to build `getBareTx()` per item. */
  api: BulletinTypedApi
  /** `undefined` for unsigned (preimage-auth) mode — emits bare extrinsics. */
  signer: PolkadotSigner | undefined
  items: UploadItem[]
  /** Indexes (into `items`) to sign in this batch. */
  indexes: number[]
  /**
   * Per-item assigned nonce — required for signed mode, ignored for
   * unsigned (each unsigned tx is independent).
   */
  itemNonce: Array<number | undefined>
  /** Mortal era anchor — the current best block. Ignored for unsigned. */
  anchor: { number: number; hash: string }
}

/**
 * Sign every item in `[fromIndex, toIndex)`. Mortal era period is 64 blocks
 * (vs. the spec's recommended 4) so a tx still validates if this handler
 * lags several blocks behind the captured anchor under concurrent load.
 */
async function signBatch(args: SignBatchArgs): Promise<string[]> {
  const { storage, api, signer, items, indexes, itemNonce, anchor } = args
  const mortality = signer
    ? {
        mortal: true as const,
        period: 64,
        startAtBlock: { height: anchor.number, hash: anchor.hash },
      }
    : undefined
  const signed: string[] = new Array(indexes.length)
  for (let k = 0; k < indexes.length; k++) {
    const i = indexes[k] as number
    const item = items[i] as UploadItem
    if (signer) {
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
      const nonce = itemNonce[i]
      if (nonce === undefined) {
        throw new Error(`signBatch: itemNonce[${i}] not assigned`)
      }
      const bytes = await tx.sign(signer, { nonce, mortality: mortality! })
      signed[k] = Binary.toHex(bytes)
    } else {
      // Unsigned (preimage-auth): build bareTx from the online api.
      // Offline storage's tx objects don't support `getBareTx()`.
      const onlineTx = isDefaultCidConfig(item)
        ? api.tx.TransactionStorage.store({ data: item.data })
        : api.tx.TransactionStorage.store_with_cid_config({
            cid: {
              codec: BigInt(item.codec ?? CidCodec.Raw),
              hashing: hashAlgorithmCodecToEnum(
                item.hashAlgo ?? HashAlgorithm.Blake2b256,
              ),
            },
            data: item.data,
          })
      const bytes = await onlineTx.getBareTx()
      signed[k] = Binary.toHex(bytes)
    }
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

/**
 * Select the prefix of `pendingIndexes` that fits in one block. Same
 * weight / length / count limits as `computeBatchEnd`, but accepts a
 * non-contiguous index list — required once hijack-recovery can leave
 * gaps in the pending set (e.g., item 5 still pending while items 6–10
 * already finalized).
 */
function selectWaveBatch(
  items: UploadItem[],
  pendingIndexes: number[],
  limits: BlockLimits,
): number[] {
  const selected: number[] = []
  let accWeight = 0n
  let accLength = 0
  for (const idx of pendingIndexes) {
    const size = items[idx]?.data.length ?? 0
    const txWeight =
      limits.storeWeightBase + limits.storeWeightPerByte * BigInt(size)
    const txLength = size + limits.extrinsicOverhead
    if (accWeight + txWeight > limits.maxNormalWeight) break
    if (accLength + txLength > limits.normalBlockLength) break
    if (selected.length >= limits.maxBlockTransactions) break
    accWeight += txWeight
    accLength += txLength
    selected.push(idx)
  }
  return selected
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
// TransactionByContentHash reads
// ---------------------------------------------------------------------------

/**
 * Read the pallet's `TransactionByContentHash[hash]` at a block. Returns
 * `(blockNumber, extrinsicIndex)` if the entry exists at that block, or
 * `undefined` if not yet stored / pruned / never stored.
 *
 * The pallet decl is `StorageMap<_, Twox64Concat, H256, (BlockNumberFor<T>, u32)>`,
 * which PAPI typically decodes as a 2-tuple. Handle alternate shapes
 * (named fields, indexed object) defensively in case metadata variants differ.
 */
async function readStoredAtBlock(
  api: BulletinTypedApi,
  contentHashHex: string,
  blockHash: string,
): Promise<{ blockNumber: number; extrinsicIndex: number } | undefined> {
  if (!api.query) return undefined
  const raw =
    await api.query.TransactionStorage.TransactionByContentHash.getValue(
      contentHashHex,
      { at: blockHash },
    )
  if (raw == null) return undefined
  if (Array.isArray(raw)) {
    return { blockNumber: Number(raw[0]), extrinsicIndex: Number(raw[1]) }
  }
  if (typeof raw === "object") {
    const r = raw as {
      0?: unknown
      1?: unknown
      block?: unknown
      index?: unknown
    }
    if (r[0] !== undefined && r[1] !== undefined) {
      return { blockNumber: Number(r[0]), extrinsicIndex: Number(r[1]) }
    }
    if (r.block !== undefined && r.index !== undefined) {
      return { blockNumber: Number(r.block), extrinsicIndex: Number(r.index) }
    }
  }
  return undefined
}

/**
 * Batch-read `TransactionByContentHash` for many content hashes at the
 * same block. Issues parallel typed-API queries; WS pipelining batches
 * them on the wire. Map keys are normalized to lower-case hex with `0x`
 * stripped so the caller can look up by the same shape regardless of
 * input casing.
 */
async function readStoredAtBlockBatch(
  api: BulletinTypedApi,
  contentHashesHex: string[],
  blockHash: string,
): Promise<Map<string, { blockNumber: number; extrinsicIndex: number }>> {
  const map = new Map<
    string,
    { blockNumber: number; extrinsicIndex: number }
  >()
  if (contentHashesHex.length === 0) return map
  const results = await Promise.all(
    contentHashesHex.map((h) =>
      readStoredAtBlock(api, h, blockHash).then((stored) => ({ h, stored })),
    ),
  )
  for (const { h, stored } of results) {
    if (stored) map.set(h.toLowerCase().replace(/^0x/, ""), stored)
  }
  return map
}

/** Normalize a hex string to lower-case without `0x` prefix for Map lookups. */
function normalizeHex(h: string): string {
  return h.toLowerCase().replace(/^0x/, "")
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
