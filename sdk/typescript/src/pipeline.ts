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
import type { BulletinTypedApi } from "./client.js"
import {
  createNonceTrackingStrategy,
  type SubmissionStrategy,
  type SubmissionStrategyKind,
} from "./submission-strategy.js"
import {
  BulletinError,
  CidCodec,
  ErrorCode,
  HashAlgorithm,
  type UploadCallback,
  type UploadItem,
  UploadStatus,
} from "./types.js"
import {
  calculateCid,
  cidToContentHashHex,
  hashAlgorithmCodecToEnum,
  isNonDefaultCidConfig,
  normalizeHex,
} from "./utils.js"

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

import type { BlockLimits } from "./types.js"

export type { BlockLimits } from "./types.js"

/** Configuration for {@link pipelineStore}. */
export interface PipelineConfig {
  /**
   * Provider factory. Called once per `pipelineStore()` invocation so each
   * outer retry gets fresh transport (dead WS connections from a failed
   * attempt are replaced). The first provider in the returned array drives
   * the chainHead monitor; every provider is used as a broadcast target
   * (pass multiple for ws-RPC redundancy).
   */
  providers: () => JsonRpcProvider[]
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
   * (in `runSignedRetry`) skips the `state_getMetadata` + offline-API
   * construction round-trip on every attempt.
   */
  bootstrap?: PipelineBootstrap
  /**
   * @internal — `runSignedRetry`-only. Pre-computed CIDs aligned with
   * `items[]` so retries skip recomputation. May be a `Promise<CID[]>` so
   * the caller can hand off computation in parallel with `pipelineStore`'s
   * bootstrap RPCs. Length is asserted to match `items.length` once
   * resolved.
   */
  precomputedCids?: CID[] | Promise<CID[]>
  /** Wire-level submission strategy. Defaults to `"nonce-tracking"`. */
  submissionStrategy?: SubmissionStrategyKind
  /**
   * @internal — `runSignedRetry`-only. Per-item nonces carried over from a
   * previous stalled `pipelineStore` call. Indices align with `items[]`.
   * An entry of `undefined` means "no carry-over, assign fresh." A defined
   * entry locks the item to that nonce so re-broadcast across the retry
   * boundary doesn't double-claim a fresh nonce and double-pay the user.
   * Callers must validate against current chainNonce before seeding —
   * stale seeds (chainNonce > seed) imply hijack/external consumption and
   * must be passed as `undefined` so the new wave assigns fresh.
   */
  seedItemNonces?: ReadonlyArray<number | undefined>
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
 * `cause` payload on a `BulletinError(STORE_STALLED)`: which items the
 * chain had already accepted when the store gave up. Callers must use
 * `finalizedIndices` (not the count) to decide what to resume — items can
 * land out of order under hijack/race conditions, so the set is not always
 * `[0..finalized)`.
 */
export interface StallCause {
  readonly finalized: number
  readonly finalizedIndices: ReadonlySet<number>
  /**
   * Snapshot of `itemNonce[]` at the moment of stall. Indices align with the
   * `items[]` array passed into the stalled `pipelineStore` call. A defined
   * entry is the nonce the SDK had committed to for that item; `undefined`
   * means no nonce was ever assigned, or the within-call hijack detector
   * cleared it. The client-level retry layer uses this to keep ownership of
   * each item's nonce slot across the retry boundary so re-broadcasts replace
   * pool entries at the same nonce rather than double-claiming above it.
   */
  readonly itemNonce: ReadonlyArray<number | undefined>
}

export function isStallError(
  err: unknown,
): err is BulletinError & { cause: StallCause } {
  return (
    err instanceof BulletinError &&
    err.code === ErrorCode.STORE_STALLED &&
    (err.cause as StallCause | undefined)?.finalizedIndices instanceof Set
  )
}

function stallError(
  finalizedIndices: ReadonlySet<number>,
  itemNonce: ReadonlyArray<number | undefined>,
  reason: string,
): BulletinError {
  return new BulletinError(
    `store stalled: ${reason}; finalized=${finalizedIndices.size}`,
    ErrorCode.STORE_STALLED,
    {
      finalized: finalizedIndices.size,
      finalizedIndices,
      itemNonce,
    } satisfies StallCause,
  )
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

/**
 * One ready-to-submit unit for the pipeline — exactly one `store` extrinsic.
 * Carries metadata plus a lazy `getData()` so the submitter fetches bytes on
 * demand (e.g. a range-read from a SeekableSource for streamed uploads) and
 * frees them after finalization, keeping resident memory bounded to the
 * in-flight window. Eager callers just return resident bytes from `getData`.
 */
export interface PipelineItem {
  /** Byte length of this item's data (without loading it). */
  size: number
  codec?: CidCodec
  hashAlgo?: HashAlgorithm
  /** Fetch the item's bytes. Called on (re-)broadcast; cached while in flight. */
  getData(): Promise<Uint8Array>
}

/** Returns true when the item should be sent via the lighter `store` extrinsic. */
/** @internal — exported for unit tests of the codec-dispatch decision. */
export function isDefaultCidConfig(item: {
  codec?: CidCodec
  hashAlgo?: HashAlgorithm
}): boolean {
  return !isNonDefaultCidConfig(
    item.codec ?? CidCodec.Raw,
    item.hashAlgo ?? HashAlgorithm.Blake2b256,
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
  items: PipelineItem[],
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

  const { providers: providersFactory, blockLimits, onEvent } = config
  const completeOn = config.completeOn ?? "finalized"
  const providers = providersFactory()
  if (providers.length === 0) {
    throw new Error(
      "pipelineStore: providers() must return at least one provider",
    )
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
      items.map(async (item) =>
        calculateCid(
          await item.getData(),
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
    { blockNumber: number; transactionIndex: number } | undefined
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
          transactionIndex: undefined as number | undefined,
        }
        onEvent({
          type: status,
          ...base,
          blockHash: lastBestBlock.hash,
          blockNumber: at.blockNumber,
          transactionIndex: storedAt[i]?.transactionIndex,
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
          transactionIndex: storedAt[i]?.transactionIndex,
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
    prefixBytes[i + 1] = (prefixBytes[i] ?? 0) + (items[i]?.size ?? 0)
  }

  // Lazy data: fetch each item's bytes on first (re-)broadcast and cache them;
  // free on finalization so resident memory tracks the in-flight window, not
  // the whole upload. For eager callers getData returns resident bytes.
  const dataCache = new Map<number, Uint8Array>()
  const loadData = async (i: number): Promise<Uint8Array> => {
    let d = dataCache.get(i)
    if (d === undefined) {
      d = await (items[i] as PipelineItem).getData()
      dataCache.set(i, d)
    }
    return d
  }
  const totalDataBytes = prefixBytes[items.length] ?? 0

  // ---------------------------------------------------------------------------
  // Connections
  // ---------------------------------------------------------------------------

  // Monitor: drives chainHead/nonce queries. It needs its OWN provider
  // instance — a getWsProvider holds a single internal socket + reconnect
  // state, so sharing one instance across two clients (monitor + submit[0])
  // makes a reconnect orphan the previous socket, which neither client's
  // destroy() can close (leaked WS after a long finalized wait). A fresh
  // factory call yields a distinct instance.
  const monitorClient = createSubstrateClient(
    providersFactory()[0] as JsonRpcProvider,
  )

  // Submission: one substrate client per provider (broadcast to all).
  const submitClients = providers.map((p) => createSubstrateClient(p))

  const startTime = Date.now()
  let startNonce = 0
  let expectedFinalNonce = 0
  let initialized = false
  let done = false
  let offlineStorage: OfflineTransactionStorage | undefined

  // Per-item assigned nonce. Undefined while an item sits in `sendQueue`;
  // set when the consumer assigns a nonce and broadcasts; cleared on
  // hijack or retryable-broadcast (item is then re-enqueued).
  const itemNonce: Array<number | undefined> = new Array(totalItems).fill(
    undefined,
  )
  // Retry budget per item — incremented each time we re-enqueue after a
  // hijack or retryable broadcast error. Caps at MAX_RETRY_ATTEMPTS;
  // beyond that we emit ItemFailed.
  const retryAttempts: number[] = new Array(totalItems).fill(0)
  // Monotonic nonce counter. Each broadcast assigns the next N consecutive
  // nonces starting here, then advances. Hijacked nonces are "wasted" —
  // never reused, the counter only moves forward.
  let nextFreeNonce = 0
  // Items whose retry budget was exhausted — these get `ItemFailed` once
  // and are excluded from subsequent waves.
  const failedItems = new Set<number>()
  const MAX_RETRY_ATTEMPTS = 10

  // FIFO queue of item indices awaiting broadcast. Populated at init with
  // every item; consumer (bestBlockChanged) drains while pool depth is
  // below target. Hijack detection and retryable broadcast errors
  // re-enqueue at the FRONT (priority over fresh items).
  const sendQueue: number[] = []

  // Submission strategy: today only nonce-tracking is implemented. The
  // `SubmissionStrategyKind` union and `config.submissionStrategy` field
  // exist so future strategies can be added without rewriting the pipeline
  // (see `docs/watch-strategy-design.md` for the watch strategy we
  // prototyped and removed).
  const _strategyKind: SubmissionStrategyKind =
    config.submissionStrategy ?? "nonce-tracking"
  const strategy: SubmissionStrategy = createNonceTrackingStrategy({
    submitClients,
  })

  // Watchdog #1: any chainHead event (`initialized`/`newBlock`/
  // `bestBlockChanged`/`finalized`/`stop`) keeps this fresh. Detects
  // total WS-stream silence — unrecoverable from our side and signals
  // the outer retry to open a new client.
  const STALL_TIMEOUT_MS = 18_000
  let lastChainEventAt = Date.now()
  let stallTimer: ReturnType<typeof setInterval> | undefined
  const touchChainEvent = (): void => {
    lastChainEventAt = Date.now()
  }

  // Watchdog #2: only refreshed when our `bestBlockChanged` or `finalized`
  // handler actually completes (post-reconcile, post-emit, post-drain).
  // Defends against the case where the chainHead WS keeps sending
  // `newBlock` heartbeats — touching watchdog #1 — but `bestBlockChanged`
  // / `finalized` stop reaching our handler (PAPI subscription drift,
  // pin-limit stalls). Bigger threshold than #1 (~5 block times) because
  // it's expected to be quieter under contention.
  const PROGRESS_TIMEOUT_MS = 30_000
  let lastProgressAt = Date.now()
  let progressTimer: ReturnType<typeof setInterval> | undefined
  const touchProgress = (): void => {
    lastProgressAt = Date.now()
  }

  // Keepalive: submit clients only broadcast, then go silent while waiting for
  // inclusion + finalization. PAPI's ws-provider abandons — without closing —
  // a socket idle past its ~40s heartbeat and reconnects, orphaning the old
  // socket; the leak survives client.destroy() (it only closes the current
  // socket) and keeps the process from exiting. A cheap periodic request holds
  // these connections under that threshold. The monitor is immune — chainHead
  // events keep it warm.
  const KEEPALIVE_INTERVAL_MS = 20_000
  let keepaliveTimer: ReturnType<typeof setInterval> | undefined

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

  // Per-block status snapshot, logged only when state changed vs previous
  // event. Surfaces quiet windows where items are waiting in pool (no
  // wave/hijack/deferred log) so the script doesn't look stuck.
  let lastStatusSig = ""

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
    strategy,
    releaseData: (i) => {
      dataCache.delete(i)
    },
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
      try {
        strategy.teardown()
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
      if (progressTimer !== undefined) {
        clearInterval(progressTimer)
        progressTimer = undefined
      }
      if (keepaliveTimer !== undefined) {
        clearInterval(keepaliveTimer)
        keepaliveTimer = undefined
      }
    }

    function failWithStall(reason: string): void {
      if (done) return
      done = true
      teardown()
      // Only items that have been emitted as completed at the requested
      // level (best-block for completeOn="best", finalized otherwise) are
      // safe to skip on resume. `storedAt` alone is insufficient — a best-
      // block observation can be reorged out before finalization, and
      // re-submitting after that is desirable.
      const completedFlags =
        completeOn === "best" ? inBlockEmitted : finalizedEmitted
      const finalizedIndices = new Set<number>()
      for (let i = 0; i < completedFlags.length; i++) {
        if (completedFlags[i]) finalizedIndices.add(i)
      }
      reject(stallError(finalizedIndices, itemNonce.slice(), reason))
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
                    if (!lastBestBlock || lastBestBlock.number < newTipNumber) {
                      lastBestBlock = { hash: lastHash, number: newTipNumber }
                    }
                    await reconcileAtBlock(api, state, lastHash)
                    recordLatency(state, Date.now(), "in_block")
                    recordLatency(state, Date.now(), "finalized")
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
                      Promise.resolve({} as { ss58Format?: number | string }),
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
                  // Consumer assigns nonces sequentially from nextFreeNonce
                  // as it drains the queue. expectedFinalNonce tracks the
                  // upper bound — initially startNonce + items.length, then
                  // grows as hijack-recovery consumes additional nonces.
                  nextFreeNonce = startNonce
                  expectedFinalNonce = startNonce + items.length
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
                contentHashesHex = cids.map(cidToContentHashHex)
                state.contentHashesHex = contentHashesHex
                // Diagnostic: log the first 12 hex chars of every content
                // hash so cross-session collisions (which would cause the
                // reconciler to read the SAME TBCH entry for two different
                // items) can be detected in test logs.
                // biome-ignore lint/suspicious/noConsole: content-hash diagnostic
                console.log(
                  `[pipelineStore] content_hashes (startNonce=${startNonce}): ${contentHashesHex.map((h, i) => `i${i}=${h.slice(0, 12)}`).join(" ")}`,
                )
                for (let i = 0; i < totalItems; i++) {
                  emit(UploadStatus.ItemStarted, i)
                  sendQueue.push(i)
                }

                // Apply seeded nonces from a prior stalled call. A seed
                // entry below the freshly-read `startNonce` means the
                // chain has already advanced past that slot without our
                // content landing — either the previous submission was
                // executed by someone else (hijack) or it was era-dropped
                // and overtaken by another tx at that nonce. Either way
                // re-broadcasting at the same nonce is impossible (nonce
                // can only be consumed once), so we discard the stale seed
                // and let the next wave assign a fresh nonce from the
                // current poolNonce floor. Surviving seeds claim their
                // original slot; re-broadcast at the same nonce with a
                // fresh era anchor either replaces the stale pool entry
                // or 1014-rejects — either way the nonce slot is consumed
                // exactly once and the user pays exactly once.
                const seed = config.seedItemNonces
                if (!unsigned && seed) {
                  for (let i = 0; i < totalItems && i < seed.length; i++) {
                    const n = seed[i]
                    if (n !== undefined && n >= startNonce) {
                      itemNonce[i] = n
                      if (n >= expectedFinalNonce) {
                        expectedFinalNonce = n + 1
                      }
                    }
                  }
                }

                // Bootstrap `lastFinalizedBlock` from chainHead's initial
                // finalized hash BEFORE flipping `initialized = true`. The
                // mortal-era anchor reads from `lastFinalizedBlock`, so the
                // first wave needs it set or we'd fall back to best block —
                // which can reorg out and invalidate every tx we sign with
                // it (BadProof at block-time validation, blocks then build
                // empty against a queued pool).
                if (!unsigned) {
                  const finNumber = await getBlockNumber(lastHash)
                  lastFinalizedBlock = { hash: lastHash, number: finNumber }
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

                // Early-out once every item is stored (or permanently
                // failed). We're just waiting on `finalized` events — no
                // need to fetch nonces or rerun the reconciler each best
                // block until then.
                if (counters.confirmed + failedItems.size >= totalItems) {
                  if (
                    completeOn === "best" &&
                    counters.confirmed >= totalItems - failedItems.size
                  ) {
                    tracking.bestAtTargetStreak += 1
                    if (tracking.bestAtTargetStreak >= 2) {
                      finish() // [TERMINATE-OK] completeOn:"best" 2-block streak
                    }
                  }
                  return
                }

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
                      monitorClient.request<number>("system_accountNextIndex", [
                        signerSs58,
                      ]),
                      readNonceAtBlock(monitorClient, signerHex, bestBlockHash),
                      getBlockNumber(bestBlockHash),
                    ])
                lastBestBlock = { hash: bestBlockHash, number: bestBlockNumber }

                // Reconcile via TBCH state at this best block — populates
                // `storedAt[i]` for items confirmed at or before B. Then
                // `emitInBlockEvents` updates `counters.confirmed` and fires
                // `ItemInBlock` with the correct `transactionIndex`.
                await reconcileAtBlock(api, state, bestBlockHash)
                recordLatency(state, Date.now(), "in_block")
                emitInBlockEvents(state)

                // Deferred verification model.
                //
                // Instead of reactive per-block hijack detection (which
                // mis-fires on transient reorgs and bounces around with
                // Deferred verification: wait until the chain has
                // executed at least as many of our account's nonces as
                // we assigned (`chainNonce >= expectedFinalNonce`). At
                // that point every in-flight item's outcome is
                // committed — either its content is in TBCH (success)
                // or another tx won the slot (hijack / era-invalidated
                // / pool-dropped — all recover the same way: clear the
                // nonce, re-enqueue, next wave assigns a fresh nonce
                // above the current pool tail).
                //
                // We compare `chainNonce` (the actual on-chain account
                // nonce read at the best block), NOT
                // `accountNextIndex` (pool-aware). accountNextIndex
                // adds pool-pending count to chainNonce — using it
                // would fire the trigger while our own txs are still
                // queued in the pool, causing every item to be falsely
                // flagged as missing and re-issued at higher nonces
                // (double submission). chainNonce only advances when a
                // tx actually executes, so seeing chainNonce reach
                // expectedFinalNonce really does mean every nonce we
                // assigned has been committed to some tx.
                if (!unsigned && chainNonce >= expectedFinalNonce) {
                  const missing: Array<{
                    i: number
                    nonce: number
                    attempts: number
                  }> = []
                  const stored: number[] = []
                  const givenUp: Array<{ i: number; nonce: number }> = []
                  for (let i = 0; i < totalItems; i++) {
                    if (failedItems.has(i)) continue
                    if (storedAt[i] !== undefined) {
                      stored.push(i)
                      continue
                    }
                    const nonce = itemNonce[i]
                    if (nonce === undefined) continue // queued, not yet submitted
                    const attempts = retryAttempts[i] ?? 0
                    if (attempts >= MAX_RETRY_ATTEMPTS) {
                      givenUp.push({ i, nonce })
                      failedItems.add(i)
                      strategy.onItemSettled(i)
                      onEvent?.({
                        type: UploadStatus.ItemFailed,
                        index: i,
                        total: totalItems,
                        cid: cids[i] as CID,
                        error: new BulletinError(
                          `item ${i}: failed to land after ${MAX_RETRY_ATTEMPTS} attempts`,
                          ErrorCode.HIJACK_BUDGET_EXCEEDED,
                        ),
                      })
                      continue
                    }
                    missing.push({ i, nonce, attempts })
                    retryAttempts[i] = attempts + 1
                    itemNonce[i] = undefined
                  }
                  if (missing.length > 0 || givenUp.length > 0) {
                    // biome-ignore lint/suspicious/noConsole: verification visibility
                    console.warn(
                      `[pipelineStore] verify @chain=${chainNonce}/exp=${expectedFinalNonce}: stored=${stored.length}, missing=${missing.length} [${missing
                        .map(
                          (m) =>
                            `i${m.i}@n${m.nonce}(att${m.attempts}->${m.attempts + 1})`,
                        )
                        .join(
                          ",",
                        )}], giveup=${givenUp.length}${givenUp.length ? ` [${givenUp.map((g) => `i${g.i}@n${g.nonce}`).join(",")}]` : ""}`,
                    )
                    const inQueue = new Set(sendQueue)
                    for (const m of missing) {
                      if (!inQueue.has(m.i)) sendQueue.unshift(m.i)
                    }
                  }
                }

                // Completion check: nothing left to broadcast and nothing
                // in flight that isn't yet stored.
                let pendingForFinalCount = 0
                for (let i = 0; i < totalItems; i++) {
                  if (storedAt[i] !== undefined) continue
                  if (failedItems.has(i)) continue
                  pendingForFinalCount++
                }
                if (pendingForFinalCount === 0) {
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

                // Consumer: drain `sendQueue` up to a block-buffer's worth.
                // selectWaveBatch internally caps at WAVE_BUFFER_BLOCKS
                // blocks of weight/length/count, so steady-state we keep
                // ~that many items in pool ahead of the block author.
                if (sendQueue.length === 0) return
                const waveIndexes = selectWaveBatch(
                  items,
                  sendQueue,
                  blockLimits,
                )
                if (waveIndexes.length === 0) return
                // Pop the consumed prefix from the queue.
                sendQueue.splice(0, waveIndexes.length)

                // Wave nonce-assignment floor: max of
                //   - `poolNonce` (chain's view: chainNonce + contiguous
                //     ready pool entries for this account). Authoritative
                //     across calls — drops on reorg/eviction so we refill
                //     freed slots, rises to avoid colliding with other
                //     clients on the same account.
                //   - one past our highest already-claimed nonce in
                //     `itemNonce[]`. Necessary because `poolNonce` lags
                //     our broadcasts (sub-second RPC propagation), so
                //     within a single best-block tick we can submit a
                //     wave at nonces N..N+k, fire wave #2 immediately,
                //     and read `poolNonce` that hasn't yet seen wave #1
                //     — without this floor, wave #2 would assign the
                //     same nonces and the pool would either 1014 or
                //     accept-and-orphan one of the conflicting items
                //     (chain consumes the nonce with the wrong content
                //     and the user pays for a hijack-like miss).
                // Items already carrying `itemNonce[i]` (re-enqueued
                // after a retryable broadcast error, or seeded from a
                // prior stalled call) keep theirs so the chain's nonce
                // sequence stays gap-free.
                //
                // `expectedFinalNonce` stays monotonic — it's the
                // deferred-verification trigger and must not drop when
                // poolNonce dips below a previously-claimed nonce.
                if (!unsigned) {
                  let ourFloor = chainNonce
                  for (let i = 0; i < itemNonce.length; i++) {
                    const n = itemNonce[i]
                    if (n !== undefined && n + 1 > ourFloor) ourFloor = n + 1
                  }
                  let next = Math.max(poolNonce, ourFloor)
                  for (let k = 0; k < waveIndexes.length; k++) {
                    const i = waveIndexes[k] as number
                    if (itemNonce[i] === undefined) {
                      itemNonce[i] = next
                      next++
                    }
                  }
                  nextFreeNonce = next
                  expectedFinalNonce = Math.max(expectedFinalNonce, next)
                }

                // Re-sign every wave: reusing prior bytes is unsafe (stale
                // era → BadProof, banned-hash on pool eviction). The wave
                // also picks up any items whose itemNonce was bumped above
                // for hijack recovery.
                //
                // Anchor the mortal era at the last *finalized* block, not
                // best. The era's signed payload includes the hash of the
                // anchor block; if that anchor reorgs out, block-time
                // validation reads a different hash at that height in the
                // canonical chain and rejects the tx with BadProof. Pool
                // then evicts and the next block builder repeats the same
                // failure, producing empty blocks against a queued pool.
                // Finalized blocks never reorg out, so the signature is
                // valid on any branch the chain takes. The 64-block era
                // period easily absorbs the ~6-block finality lag.
                //
                // `lastFinalizedBlock` is bootstrapped during initialized
                // (before this wave can fire) and refreshed on every
                // `finalized` chainHead event. Unsigned waves pass
                // `bestBlock` as a no-op — signBatch ignores anchor when
                // signer is undefined.
                const eraAnchor =
                  !unsigned && lastFinalizedBlock
                    ? lastFinalizedBlock
                    : { number: bestBlockNumber, hash: bestBlockHash }
                const signed = await signBatch({
                  storage: offlineStorage!,
                  api,
                  signer,
                  items,
                  loadData,
                  indexes: waveIndexes,
                  itemNonce,
                  anchor: eraAnchor,
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
                }

                const {
                  terminalCode: waveTerminalCode,
                  terminalMsg: waveTerminalMsg,
                  retryableCount: waveRetryableCount,
                  retryableLastCode: waveRetryableLastCode,
                  itemResults,
                } = await strategy.broadcastWave({
                  signed,
                  waveIndexes,
                  counters,
                })
                counters.waves++

                // Capture wave's nonce range BEFORE possible re-enqueue
                // (which may clear itemNonce for fresh-nonce retries).
                const firstNonce =
                  !unsigned && waveIndexes.length > 0
                    ? itemNonce[waveIndexes[0] as number]
                    : undefined
                const lastNonce =
                  !unsigned && waveIndexes.length > 0
                    ? itemNonce[waveIndexes[waveIndexes.length - 1] as number]
                    : undefined

                // Per-item disposition of every wave entry. Two reject
                // classes are handled here:
                //
                //   retryable (1014/1016, FutureTransaction, TooLowPriority):
                //     Pool was full or our priority lost. Re-broadcast at
                //     the SAME nonce next wave — re-assigning a higher
                //     nonce would create a gap in the account's nonce
                //     sequence and same-account txs are processed in
                //     nonce order. Doesn't burn the retry budget (pool
                //     pressure isn't a hijack).
                //
                //   terminal (1010 InvalidTransaction etc.):
                //     The slot is unusable — most commonly a concurrent
                //     same-account uploader grabbed the nonce, but could
                //     also be a stale era or bad proof. Clear nonce, get
                //     a fresh one next drain, increment retry budget.
                //     We do NOT bail the wave (only individual items are
                //     affected); the outer retry layer still catches
                //     budget exhaustion via failedItems.
                const reRejected: number[] = []
                if (!unsigned) {
                  for (let k = 0; k < waveIndexes.length; k++) {
                    const i = waveIndexes[k] as number
                    const result = itemResults[k]
                    if (result?.accepted) continue
                    // Was this terminal or retryable?
                    const code = result?.retryableCode
                    if (code === undefined) {
                      // No retryable code recorded means every client
                      // returned terminal. Treat as per-item terminal —
                      // refresh the nonce so we don't loop on the same
                      // poisoned slot.
                      const attempts = retryAttempts[i] ?? 0
                      if (attempts >= MAX_RETRY_ATTEMPTS) {
                        // biome-ignore lint/suspicious/noConsole: visibility for permanent failure
                        console.warn(
                          `[pipelineStore] item ${i}: terminal RPC error ${MAX_RETRY_ATTEMPTS} times (nonce ${itemNonce[i]}); giving up`,
                        )
                        failedItems.add(i)
                        strategy.onItemSettled(i)
                        onEvent?.({
                          type: UploadStatus.ItemFailed,
                          index: i,
                          total: totalItems,
                          cid: cids[i] as CID,
                          error: new BulletinError(
                            `item ${i}: terminal RPC error after ${MAX_RETRY_ATTEMPTS} attempts`,
                            ErrorCode.TRANSACTION_FAILED,
                          ),
                        })
                        continue
                      }
                      itemNonce[i] = undefined
                      retryAttempts[i] = attempts + 1
                      reRejected.push(i)
                    } else {
                      // Retryable: keep nonce, just re-broadcast next wave.
                      reRejected.push(i)
                    }
                  }
                } else {
                  // Unsigned: same intent, but no nonces. Just re-enqueue
                  // anything not accepted by the pool.
                  for (let k = 0; k < waveIndexes.length; k++) {
                    const i = waveIndexes[k] as number
                    if (itemResults[k]?.accepted) {
                      broadcastedItems.add(i)
                    } else {
                      reRejected.push(i)
                    }
                  }
                }
                if (reRejected.length > 0) sendQueue.unshift(...reRejected)

                // Visibility for the producer/consumer queue: one line per
                // wave showing how many items were broadcast, the nonce
                // range used, what's left in the queue, and the per-item
                // index→nonce mapping for cross-correlation with the
                // verification log.
                if (!unsigned && waveIndexes.length > 0) {
                  const mapping = waveIndexes
                    .map(
                      (i) =>
                        `i${i}@n${itemNonce[i]}${
                          retryAttempts[i] ? `(r${retryAttempts[i]})` : ""
                        }`,
                    )
                    .join(",")
                  // biome-ignore lint/suspicious/noConsole: queue-drain visibility
                  console.log(
                    `[pipelineStore] wave #${counters.waves}: broadcast ${waveIndexes.length} items (nonces ${firstNonce}..${lastNonce}) [${mapping}]; queue=${sendQueue.length} expFinal=${expectedFinalNonce}`,
                  )
                }

                // Wave-level terminal is now only logged for diagnostics —
                // per-item handling above already re-enqueued or failed
                // the affected items. No more whole-wave abort on a single
                // 1010 (which under concurrent same-account uploads is
                // routine).
                if (waveTerminalCode !== undefined) {
                  // biome-ignore lint/suspicious/noConsole: diagnostic for terminal codes seen in wave
                  console.warn(
                    `[pipelineStore] wave #${counters.waves}: terminal RPC code observed (code=${waveTerminalCode}, msg=${waveTerminalMsg?.slice(0, 120)}); affected items re-enqueued with fresh nonces`,
                  )
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

                // Quiet-window visibility: emit a status snapshot when the
                // pipeline is between actions (nothing to broadcast, items
                // pending in pool). Skipped while the signature is the
                // same as the previous event so a steady-state wait
                // produces ONE line, not one per block.
                if (!unsigned) {
                  let inflightCount = 0
                  let storedCount = 0
                  for (let i = 0; i < totalItems; i++) {
                    if (storedAt[i] !== undefined) {
                      storedCount++
                    } else if (
                      itemNonce[i] !== undefined &&
                      !failedItems.has(i)
                    ) {
                      inflightCount++
                    }
                  }
                  const sig = `q=${sendQueue.length},pool=${inflightCount},stored=${storedCount},failed=${failedItems.size},chain=${chainNonce},next=${nextFreeNonce}`
                  if (
                    sig !== lastStatusSig &&
                    (sendQueue.length > 0 || inflightCount > 0)
                  ) {
                    // biome-ignore lint/suspicious/noConsole: quiet-window visibility
                    console.log(`[pipelineStore] #${bestBlockNumber} ${sig}`)
                    lastStatusSig = sig
                  }
                }
                // Progress watchdog: handler completed successfully.
                touchProgress()
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

                // Queue model: completion is purely TBCH-driven (storedAt
                // set for every item), so we no longer need the account
                // nonce at the finalized block.
                const finBlockNumber = await getBlockNumber(lastHash)
                lastFinalizedBlock = { hash: lastHash, number: finBlockNumber }

                // Reconcile at the finalized block — populates `storedAt[i]`
                // for any item whose TBCH entry exists at finalization. Then
                // emit ItemFinalized (monotonic) with `transactionIndex`.
                await reconcileAtBlock(api, state, lastHash)
                recordLatency(state, Date.now(), "finalized")
                emitFinalizedEvents(state)

                // Completion: every input item is either stored or
                // permanently failed (hijack budget exceeded). The
                // queue model guarantees items are re-enqueued on every
                // pool rejection / hijack, so `storedAt` is the
                // authoritative signal — no nonce-based fallback
                // needed (it would over-trigger when items sit in
                // `sendQueue` between broadcasts).
                if (counters.finalized + failedItems.size >= items.length) {
                  // Pre-finish summary: per-item state classification so
                  // we can audit the chain accounting after the test.
                  // biome-ignore lint/suspicious/noConsole: completion summary
                  const stored: number[] = []
                  const failed: number[] = []
                  const orphan: number[] = []
                  for (let i = 0; i < totalItems; i++) {
                    if (failedItems.has(i)) failed.push(i)
                    else if (storedAt[i] !== undefined) stored.push(i)
                    else orphan.push(i)
                  }
                  // biome-ignore lint/suspicious/noConsole: completion summary
                  console.log(
                    `[pipelineStore] finish @finalized=${finBlockNumber}: stored=${stored.length} [${stored.map((i) => `i${i}@blk${storedAt[i]?.blockNumber}`).join(",")}], failed=${failed.length}${failed.length ? ` [${failed.join(",")}]` : ""}${orphan.length ? `, ORPHAN=${orphan.length} [${orphan.join(",")}]` : ""}`,
                  )
                  finish() // [TERMINATE-OK] all items finalized or failed
                }
                // Progress watchdog: handler completed successfully.
                touchProgress()
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

    progressTimer = setInterval(() => {
      if (done) return
      if (Date.now() - lastProgressAt > PROGRESS_TIMEOUT_MS) {
        // [TERMINATE-STALL] no bestBlock/finalized handler progress.
        // Outer retry will reopen a fresh client and TBCH-dedup any
        // items that landed but never got finalized through this
        // subscription.
        failWithStall(
          `no bestBlock/finalized progress for ${PROGRESS_TIMEOUT_MS} ms`,
        )
      }
    }, 6_000)
    progressTimer.unref?.()

    keepaliveTimer = setInterval(() => {
      if (done) return
      for (const c of submitClients) {
        c.request("system_health", []).catch(() => {})
      }
    }, KEEPALIVE_INTERVAL_MS)
    keepaliveTimer.unref?.()
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
  items: PipelineItem[]
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
   * `transactionIndex` is surfaced on `ItemInBlock` / `ItemFinalized` events.
   */
  storedAt: Array<{ blockNumber: number; transactionIndex: number } | undefined>
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
  /** Submission strategy — notified when an item is settled at finality. */
  strategy: SubmissionStrategy
  /** Release an item's cached bytes once finalized (frees in-flight memory). */
  releaseData: (i: number) => void
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
  // Re-check every signed/broadcast item — items whose `storedAt` was set
  // at a previous best block may have been reorged out at this block.
  // The reconciler is the only source of truth for storedAt; if TBCH no
  // longer shows our entry, clear it so emitInBlockEvents can retract.
  const considered: number[] = []
  for (let i = 0; i < state.totalItems; i++) {
    if (state.submissionAnchorBlock[i] !== undefined) considered.push(i)
  }
  if (considered.length === 0) return
  const hashes = considered.map((i) => state.contentHashesHex[i] as string)
  const tbch = await readStoredAtBlockBatch(api, hashes, blockHash)
  for (let k = 0; k < considered.length; k++) {
    const i = considered[k]!
    const entry = tbch.get(normalizeHex(hashes[k] as string))
    const anchor = state.submissionAnchorBlock[i]!
    if (entry && entry.blockNumber >= anchor) {
      // Log only first-time set, to keep logs tractable
      if (state.storedAt[i] === undefined) {
        // biome-ignore lint/suspicious/noConsole: stored-set diagnostic
        console.log(
          `[reconcile] item ${i}: storedAt SET blk=${entry.blockNumber} idx=${entry.transactionIndex} (anchor=${anchor}, hash=${hashes[k]?.slice(0, 12)}, queried=${blockHash.slice(0, 10)})`,
        )
      }
      state.storedAt[i] = entry
    } else if (state.storedAt[i] !== undefined) {
      // Was stored, no longer present at this block — reorg or
      // out-of-retention removal. Caller handles re-emission via the
      // `inBlockEmitted` flag.
      // biome-ignore lint/suspicious/noConsole: reorg-out visibility
      console.warn(
        `[reconcile] item ${i}: storedAt cleared at block ${blockHash.slice(0, 10)} (previous=blk${state.storedAt[i]?.blockNumber}, anchor=${anchor}, tbch=${entry?.blockNumber ?? "missing"})`,
      )
      state.storedAt[i] = undefined
    }
  }
}

/**
 * Push latency entries for items now confirmed but not yet emitted at the
 * given lifecycle stage (best-block inclusion or finalization). Run before
 * the matching `emit*Events` so the `!emitted` flag is still set.
 */
function recordLatency(
  s: WaveState,
  observedAt: number,
  kind: "in_block" | "finalized",
): void {
  const emitted = kind === "in_block" ? s.inBlockEmitted : s.finalizedEmitted
  const latencies =
    kind === "in_block" ? s.inclusionLatenciesMs : s.finalizationLatenciesMs
  for (let i = 0; i < s.totalItems; i++) {
    if (s.storedAt[i] === undefined || emitted[i]) continue
    const broadcast = s.broadcastAtMs[i]
    if (broadcast !== undefined) latencies.push(observedAt - broadcast)
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
        s.strategy.onItemSettled(i)
        s.releaseData(i)
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
  items: PipelineItem[]
  /** Fetch an item's bytes (cache-backed; loads lazily on first use). */
  loadData: (i: number) => Promise<Uint8Array>
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
  const { storage, api, signer, items, loadData, indexes, itemNonce, anchor } =
    args
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
    const item = items[i] as PipelineItem
    const data = await loadData(i)
    if (signer) {
      const tx = isDefaultCidConfig(item)
        ? storage.store({ data })
        : storage.store_with_cid_config({
            cid: {
              codec: BigInt(item.codec ?? CidCodec.Raw),
              hashing: hashAlgorithmCodecToEnum(
                item.hashAlgo ?? HashAlgorithm.Blake2b256,
              ),
            },
            data,
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
        ? api.tx.TransactionStorage.store({ data })
        : api.tx.TransactionStorage.store_with_cid_config({
            cid: {
              codec: BigInt(item.codec ?? CidCodec.Raw),
              hashing: hashAlgorithmCodecToEnum(
                item.hashAlgo ?? HashAlgorithm.Blake2b256,
              ),
            },
            data,
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
  items: PipelineItem[],
  fromIndex: number,
  limits: BlockLimits,
): number {
  let toIndex = fromIndex
  let accWeight = 0n
  let accLength = 0

  while (toIndex < items.length) {
    const size = items[toIndex]?.size ?? 0
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
 * How many blocks of pool buffer to maintain per wave. Two blocks gives
 * the next block author non-empty pool depth even when our previous
 * wave's broadcasts haven't fully propagated across collators — the
 * single-block buffer used to leave gaps (one full block followed by
 * sparse / empty ones) because gossip lag from collator-A to collator-B
 * could exceed the ~6 s block time. Items deferred by per-account
 * future-tx caps (1014) re-broadcast at the same nonce on the next
 * wave thanks to sticky `itemNonce`, so the larger wave doesn't strand
 * pool slots.
 */
const WAVE_BUFFER_BLOCKS = 2

/**
 * Select the prefix of `pendingIndexes` that fits in `bufferBlocks`
 * blocks. Same weight / length / count limits as `computeBatchEnd`, but
 * accepts a non-contiguous index list — required once hijack-recovery
 * can leave gaps in the pending set (e.g., item 5 still pending while
 * items 6–10 already finalized).
 */
function selectWaveBatch(
  items: PipelineItem[],
  pendingIndexes: number[],
  limits: BlockLimits,
  bufferBlocks: number = WAVE_BUFFER_BLOCKS,
): number[] {
  const maxWeight = limits.maxNormalWeight * BigInt(bufferBlocks)
  const maxLength = limits.normalBlockLength * bufferBlocks
  const maxTxs = limits.maxBlockTransactions * bufferBlocks
  const selected: number[] = []
  let accWeight = 0n
  let accLength = 0
  for (const idx of pendingIndexes) {
    const size = items[idx]?.size ?? 0
    const txWeight =
      limits.storeWeightBase + limits.storeWeightPerByte * BigInt(size)
    const txLength = size + limits.extrinsicOverhead
    if (accWeight + txWeight > maxWeight) break
    if (accLength + txLength > maxLength) break
    if (selected.length >= maxTxs) break
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
 * `(blockNumber, transactionIndex)` if the entry exists at that block, or
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
): Promise<{ blockNumber: number; transactionIndex: number } | undefined> {
  return readStoredAt(api, contentHashHex, blockHash)
}

/**
 * Same as `readStoredAtBlock` but accepts either a specific block hash
 * (only safe when the caller knows PAPI's chainHead has that block
 * pinned — e.g. the active follower's current best/finalized hash) or
 * the PAPI sentinel `"finalized"` / `"best"`. Sentinels resolve through
 * `chainHead_v1_storage` against a tracked block, so the read never
 * falls back to `archive_v1_storage` and never UnknownBlock-fails on
 * non-archive nodes. Passing an arbitrary external hash (e.g. one
 * fetched via legacy `chain_getFinalizedHead`) forces the archive
 * fallback and is the foot-gun this signature steers callers away from.
 */
export async function readStoredAt(
  api: BulletinTypedApi,
  contentHashHex: string,
  at: string = "finalized",
): Promise<{ blockNumber: number; transactionIndex: number } | undefined> {
  if (!api.query) return undefined
  const raw =
    await api.query.TransactionStorage.TransactionByContentHash.getValue(
      contentHashHex,
      { at },
    )
  if (raw == null) return undefined
  if (Array.isArray(raw)) {
    return { blockNumber: Number(raw[0]), transactionIndex: Number(raw[1]) }
  }
  if (typeof raw === "object") {
    const r = raw as {
      0?: unknown
      1?: unknown
      block?: unknown
      index?: unknown
    }
    if (r[0] !== undefined && r[1] !== undefined) {
      return { blockNumber: Number(r[0]), transactionIndex: Number(r[1]) }
    }
    if (r.block !== undefined && r.index !== undefined) {
      return { blockNumber: Number(r.block), transactionIndex: Number(r.index) }
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
): Promise<Map<string, { blockNumber: number; transactionIndex: number }>> {
  const map = new Map<
    string,
    { blockNumber: number; transactionIndex: number }
  >()
  if (contentHashesHex.length === 0) return map
  const results = await Promise.all(
    contentHashesHex.map((h) =>
      readStoredAtBlock(api, h, blockHash).then((stored) => ({ h, stored })),
    ),
  )
  for (const { h, stored } of results) {
    if (stored) map.set(normalizeHex(h), stored)
  }
  return map
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
