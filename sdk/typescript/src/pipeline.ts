// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * High-throughput pipeline submitter for bulk data storage on Bulletin Chain.
 *
 * Ported from the stress-test's producer/consumer pipeline architecture.
 * Uses fire-and-forget (`author_submitExtrinsic`) for maximum throughput
 * with N worker connections, round-robin dispatch, and backpressure.
 *
 * @packageDocumentation
 */

import type { JsonRpcProvider } from "@polkadot-api/json-rpc-provider"
import {
  createClient as createSubstrateClient,
  type SubstrateClient,
} from "@polkadot-api/substrate-client"
import { Binary, type PolkadotSigner } from "polkadot-api"

import type { BulletinTypedApi } from "./async-client.js"

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** Configuration for the pipeline submitter. */
export interface PipelineConfig {
  /** RPC WebSocket URLs to distribute workers across. */
  wsUrls: string[]
  /**
   * Factory that creates a {@link JsonRpcProvider} from a URL.
   *
   * Callers supply the environment-appropriate provider:
   * ```ts
   * import { getWsProvider } from "polkadot-api/ws-provider/node"
   * import { withPolkadotSdkCompat } from "polkadot-api/polkadot-sdk-compat"
   *
   * const config: PipelineConfig = {
   *   wsUrls: ["wss://node-0.example.com"],
   *   createProvider: (url) => withPolkadotSdkCompat(getWsProvider(url)),
   * }
   * ```
   */
  createProvider: (url: string) => JsonRpcProvider
  /** Number of submission workers (default: max(wsUrls.length * 2, 8)). */
  workers?: number
  /**
   * Backpressure threshold — the dispatcher pauses when
   * `submitted - confirmed` exceeds this value (default: 4000).
   */
  backpressureThreshold?: number
  /** Progress callback fired on every new block confirmation. */
  onProgress?: (stats: PipelineStats) => void
}

/** Snapshot of pipeline progress. */
export interface PipelineStats {
  submitted: number
  confirmed: number
  errors: number
  poolFullRetries: number
  staleNonces: number
  elapsedMs: number
  throughputBytesPerSec: number
  txPerSec: number
}

/** Final pipeline result. */
export interface PipelineResult extends PipelineStats {
  totalSubmitted: number
  totalConfirmed: number
  totalErrors: number
  durationMs: number
  totalBytes: number
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/** Classified transaction-pool / RPC error (mirrors stress-test). */
const enum TxPoolError {
  PoolFull,
  Banned,
  ExhaustsResources,
  ConnectionDead,
  TxDropped,
  AlreadyImported,
  StaleNonce,
  FutureNonce,
  Other,
}

interface WorkItem {
  signedHex: string
  contentHash: string
  dataSize: number
}

// ---------------------------------------------------------------------------
// Error classification (ported from stress-test/src/store.rs:201-259)
// ---------------------------------------------------------------------------

function classifyTxError(error: unknown): TxPoolError {
  const msg = String(
    error instanceof Error ? error.message : error,
  ).toLowerCase()

  // Pool full (1016) or priority too low (1014)
  if (
    msg.includes("1016") ||
    msg.includes("immediately dropped") ||
    msg.includes("1014") ||
    msg.includes("priority is too low")
  ) {
    return TxPoolError.PoolFull
  }

  // Transaction dropped (entered pool then evicted)
  if (msg.includes("transaction dropped") || msg.includes("was dropped")) {
    return TxPoolError.TxDropped
  }

  // Already imported (1013)
  if (msg.includes("1013") || msg.includes("already imported")) {
    return TxPoolError.AlreadyImported
  }

  // Temporarily banned (1012)
  if (msg.includes("1012") || msg.includes("temporarily banned")) {
    return TxPoolError.Banned
  }

  // Stale nonce
  if (msg.includes("stale") || (msg.includes("1010") && msg.includes("outdated"))) {
    return TxPoolError.StaleNonce
  }

  // Future nonce (1021)
  if (
    msg.includes("1021") ||
    (msg.includes("1010") && msg.includes("future")) ||
    msg.includes("will be valid in the future")
  ) {
    return TxPoolError.FutureNonce
  }

  // Exhausts resources
  if (msg.includes("exhaust")) {
    return TxPoolError.ExhaustsResources
  }

  // Connection-level errors
  if (
    msg.includes("connection reset") ||
    msg.includes("background task closed") ||
    msg.includes("connection closed") ||
    msg.includes("broken pipe") ||
    msg.includes("restart required") ||
    msg.includes("not connected") ||
    msg.includes("i/o error") ||
    msg.includes("websocket") ||
    msg.includes("socket hang up") ||
    msg.includes("econnrefused") ||
    msg.includes("econnreset")
  ) {
    return TxPoolError.ConnectionDead
  }

  // Generic invalid tx (1010) — most common cause is stale nonce
  if (msg.includes("1010") || msg.includes("invalid transaction")) {
    return TxPoolError.StaleNonce
  }

  return TxPoolError.Other
}

// ---------------------------------------------------------------------------
// BoundedChannel — lightweight async bounded queue
// ---------------------------------------------------------------------------

interface Waiter<T> {
  resolve: (value: T) => void
}

class BoundedChannel<T> {
  private buffer: T[] = []
  private closed = false
  private senderWaiters: Waiter<void>[] = []
  private receiverWaiters: Waiter<T | null>[] = []

  constructor(private readonly capacity: number) {}

  /** Non-blocking send. Returns true if the item was enqueued. */
  trySend(item: T): boolean {
    if (this.closed) return false

    // If a receiver is waiting, deliver directly
    if (this.receiverWaiters.length > 0) {
      const waiter = this.receiverWaiters.shift()!
      waiter.resolve(item)
      return true
    }

    if (this.buffer.length < this.capacity) {
      this.buffer.push(item)
      return true
    }
    return false
  }

  /** Blocking send — resolves when a slot is available or channel closes. */
  async send(item: T): Promise<boolean> {
    if (this.trySend(item)) return true
    if (this.closed) return false

    // Wait for capacity
    await new Promise<void>((resolve) => {
      this.senderWaiters.push({ resolve })
    })

    if (this.closed) return false
    return this.trySend(item) || false
  }

  /** Receive the next item. Returns null when channel is closed and empty. */
  async recv(): Promise<T | null> {
    if (this.buffer.length > 0) {
      const item = this.buffer.shift()!
      // Wake a blocked sender
      if (this.senderWaiters.length > 0) {
        const waiter = this.senderWaiters.shift()!
        waiter.resolve()
      }
      return item
    }

    if (this.closed) return null

    return new Promise<T | null>((resolve) => {
      this.receiverWaiters.push({ resolve })
    })
  }

  /** Close the channel. Pending receivers get null. */
  close(): void {
    this.closed = true
    for (const w of this.receiverWaiters) w.resolve(null)
    this.receiverWaiters = []
    for (const w of this.senderWaiters) w.resolve()
    this.senderWaiters = []
  }
}

// ---------------------------------------------------------------------------
// Shared stats
// ---------------------------------------------------------------------------

class SharedStats {
  submitted = 0
  submittedBytes = 0
  confirmed = 0
  errors = 0
  poolFullRetries = 0
  staleNonces = 0

  private blockListeners: Array<() => void> = []

  /** Notify waiters that a new block arrived (confirmation count may have changed). */
  notifyBlock(): void {
    const listeners = this.blockListeners.splice(0)
    for (const fn of listeners) fn()
  }

  /** Wait for the next block notification (with timeout). */
  waitForBlock(timeoutMs: number): Promise<void> {
    return new Promise<void>((resolve) => {
      const timer = setTimeout(() => {
        const idx = this.blockListeners.indexOf(resolve)
        if (idx >= 0) this.blockListeners.splice(idx, 1)
        resolve()
      }, timeoutMs)
      this.blockListeners.push(() => {
        clearTimeout(timer)
        resolve()
      })
    })
  }
}

// ---------------------------------------------------------------------------
// StoreWorker
// ---------------------------------------------------------------------------

class StoreWorker {
  private consecutiveConnErrors = 0

  constructor(
    private readonly workerId: number,
    private client: SubstrateClient,
    private readonly reconnectUrl: string,
    private readonly createProvider: (url: string) => JsonRpcProvider,
    private readonly stats: SharedStats,
  ) {}

  /** Submit one pre-signed extrinsic with retry logic. */
  async submit(signedHex: string): Promise<void> {
    for (;;) {
      try {
        await this.client.request<string>(
          "author_submitExtrinsic",
          [signedHex],
        )
        this.stats.submitted += 1
        this.consecutiveConnErrors = 0
        return
      } catch (e: unknown) {
        const cls = classifyTxError(e)

        switch (cls) {
          case TxPoolError.PoolFull:
            this.stats.poolFullRetries += 1
            this.consecutiveConnErrors = 0
            await sleep(1000)
            break // retry

          case TxPoolError.Banned:
          case TxPoolError.ExhaustsResources:
            this.stats.poolFullRetries += 1
            this.consecutiveConnErrors = 0
            await this.stats.waitForBlock(3000)
            break // retry

          case TxPoolError.ConnectionDead:
            this.consecutiveConnErrors += 1
            if (this.consecutiveConnErrors >= 60) {
              this.stats.errors += 1
              throw new Error(
                `pipeline worker ${this.workerId}: reconnect failed after 60 attempts`,
              )
            }
            {
              const backoffMs = Math.min(
                1000 * 2 ** this.consecutiveConnErrors,
                30_000,
              )
              await sleep(backoffMs)
              try {
                this.client.destroy()
              } catch {
                /* ignore cleanup errors */
              }
              try {
                this.client = createSubstrateClient(
                  this.createProvider(this.reconnectUrl),
                )
                this.consecutiveConnErrors = 0
              } catch {
                /* will retry on next loop */
              }
            }
            break // retry

          case TxPoolError.TxDropped:
          case TxPoolError.AlreadyImported:
            this.consecutiveConnErrors = 0
            // Nonce may have been consumed — count as submitted
            this.stats.submitted += 1
            return

          case TxPoolError.StaleNonce:
            this.consecutiveConnErrors = 0
            this.stats.staleNonces += 1
            this.stats.submitted += 1
            return

          case TxPoolError.FutureNonce:
            this.consecutiveConnErrors = 0
            this.stats.errors += 1
            return

          case TxPoolError.Other:
          default:
            this.consecutiveConnErrors = 0
            this.stats.errors += 1
            return
        }
      }
    }
  }

  destroy(): void {
    try {
      this.client.destroy()
    } catch {
      /* ignore */
    }
  }
}

// ---------------------------------------------------------------------------
// Nonce-based confirmation monitor
// ---------------------------------------------------------------------------

/**
 * Polls `system_accountNextIndex` to track how many transactions have been
 * confirmed on-chain. Uses a 2-second polling interval — fast enough for
 * backpressure responsiveness without hammering the RPC.
 */
async function runNonceMonitor(
  monitorClient: SubstrateClient,
  accountAddress: string,
  startNonce: number,
  totalItems: number,
  stats: SharedStats,
  signal: AbortSignal,
): Promise<void> {
  async function queryNonce(): Promise<void> {
    if (signal.aborted) return
    try {
      const currentNonce = await monitorClient.request<number>(
        "system_accountNextIndex",
        [accountAddress],
      )
      const newConfirmed = Math.max(0, currentNonce - startNonce)
      stats.confirmed = Math.min(newConfirmed, totalItems)
      stats.notifyBlock()
    } catch {
      /* ignore query errors — will retry on next poll */
    }
  }

  // Initial query
  await queryNonce()

  // Poll every 2 seconds until aborted
  while (!signal.aborted) {
    await sleep(2_000)
    await queryNonce()
  }
}

// ---------------------------------------------------------------------------
// Dispatcher
// ---------------------------------------------------------------------------

const DEFAULT_BACKPRESSURE_THRESHOLD = 4000
const WORKER_CHANNEL_CAPACITY = 2

/** Round-robin dispatch: try non-blocking on each, fall back to blocking. */
async function dispatchToWorkers(
  item: WorkItem,
  channels: BoundedChannel<WorkItem>[],
  roundRobin: { value: number },
): Promise<void> {
  const n = channels.length
  for (let attempt = 0; attempt < n; attempt++) {
    const i = (roundRobin.value + attempt) % n
    if (channels[i]!.trySend(item)) {
      roundRobin.value = (i + 1) % n
      return
    }
  }
  // All channels full — blocking send on the round-robin channel
  const i = roundRobin.value % n
  await channels[i]!.send(item)
  roundRobin.value = (i + 1) % n
}

// ---------------------------------------------------------------------------
// Main pipeline function
// ---------------------------------------------------------------------------

/**
 * Submit multiple data items through a high-throughput fire-and-forget pipeline.
 *
 * Pre-signs all transactions with sequential nonces, distributes them across
 * N worker connections via round-robin, and monitors confirmation via nonce
 * queries on each new block.
 *
 * @example
 * ```ts
 * import { getWsProvider } from "polkadot-api/ws-provider/node"
 * import { withPolkadotSdkCompat } from "polkadot-api/polkadot-sdk-compat"
 *
 * const result = await pipelineStore(typedApi, signer, dataItems, {
 *   wsUrls: ["wss://node-0.example.com", "wss://node-1.example.com"],
 *   createProvider: (url) => withPolkadotSdkCompat(getWsProvider(url)),
 *   onProgress: (stats) => console.log(`${stats.confirmed}/${dataItems.length}`),
 * })
 * ```
 */
export async function pipelineStore(
  api: BulletinTypedApi,
  signer: PolkadotSigner,
  items: Uint8Array[],
  config: PipelineConfig,
  signal?: AbortSignal,
): Promise<PipelineResult> {
  if (items.length === 0) {
    return emptyResult()
  }

  const {
    wsUrls,
    createProvider,
    backpressureThreshold = DEFAULT_BACKPRESSURE_THRESHOLD,
    onProgress,
  } = config

  if (wsUrls.length === 0) {
    throw new Error("pipelineStore: at least one wsUrl is required")
  }

  const numWorkers = config.workers ?? Math.max(wsUrls.length * 2, 8)

  // Abort controller for internal shutdown
  const abortController = new AbortController()
  const internalSignal = abortController.signal
  if (signal) {
    signal.addEventListener("abort", () => abortController.abort(), {
      once: true,
    })
  }

  const stats = new SharedStats()
  const startTime = Date.now()
  let totalBytes = 0

  // -----------------------------------------------------------------------
  // 1. Create monitor client and fetch starting nonce
  // -----------------------------------------------------------------------
  const monitorClient = createSubstrateClient(
    createProvider(wsUrls[0]!),
  )

  // Get the signer's SS58 address for nonce queries
  const signerAddress = await getSignerAddress(signer, api)

  const startNonce = await monitorClient.request<number>(
    "system_accountNextIndex",
    [signerAddress],
  )

  // Start the nonce monitor
  const monitorDone = runNonceMonitor(
    monitorClient,
    signerAddress,
    startNonce,
    items.length,
    stats,
    internalSignal,
  )

  // -----------------------------------------------------------------------
  // 2. Create worker connections and channels
  // -----------------------------------------------------------------------
  const workerClients: SubstrateClient[] = []
  const workers: StoreWorker[] = []
  const channels: BoundedChannel<WorkItem>[] = []
  const workerDone: Promise<void>[] = []

  for (let i = 0; i < numWorkers; i++) {
    const url = wsUrls[i % wsUrls.length]!
    const client = createSubstrateClient(createProvider(url))
    workerClients.push(client)

    const worker = new StoreWorker(i, client, url, createProvider, stats)
    workers.push(worker)

    const channel = new BoundedChannel<WorkItem>(WORKER_CHANNEL_CAPACITY)
    channels.push(channel)

    // Worker loop: consume from channel
    workerDone.push(
      (async () => {
        for (;;) {
          const item = await channel.recv()
          if (item === null) break
          await worker.submit(item.signedHex)
        }
      })(),
    )
  }

  // -----------------------------------------------------------------------
  // 3. Sign and dispatch loop
  // -----------------------------------------------------------------------
  const roundRobin = { value: 0 }
  let dispatchedSinceBackpressure = 0
  let nonce = startNonce

  try {
    for (const data of items) {
      if (internalSignal.aborted) break

      // Backpressure check
      if (dispatchedSinceBackpressure >= backpressureThreshold) {
        await waitForBackpressure(stats, backpressureThreshold)
        dispatchedSinceBackpressure = 0
      }

      // Create and sign the store transaction
      const tx = api.tx.TransactionStorage.store({
        data: Binary.fromBytes(data),
      })

      // The `sign` method exists on PAPI Transaction objects but is not in
      // the SDK's minimal PapiTransaction interface. Cast through unknown.
      const signedHex = await (tx as unknown as SignableTransaction).sign(
        signer,
        { nonce: nonce++ },
      )

      const dataSize = data.length
      totalBytes += dataSize

      const item: WorkItem = {
        signedHex,
        contentHash: "", // Not needed for nonce-based tracking
        dataSize,
      }

      await dispatchToWorkers(item, channels, roundRobin)
      dispatchedSinceBackpressure += 1

      // Emit progress periodically (every 16 items)
      if (onProgress && dispatchedSinceBackpressure % 16 === 0) {
        emitProgress(stats, startTime, totalBytes, onProgress)
      }
    }
  } finally {
    // -----------------------------------------------------------------------
    // 4. Shutdown
    // -----------------------------------------------------------------------
    // Close all worker channels so workers drain and exit
    for (const ch of channels) ch.close()

    // Wait for workers to finish (with timeout)
    await Promise.race([
      Promise.allSettled(workerDone),
      sleep(10_000),
    ])

    // Wait for confirmations to catch up (up to 60s)
    const shutdownDeadline = Date.now() + 60_000
    while (
      stats.confirmed < stats.submitted &&
      Date.now() < shutdownDeadline &&
      !internalSignal.aborted
    ) {
      await stats.waitForBlock(3_000)
    }

    // Final progress emit
    if (onProgress) {
      emitProgress(stats, startTime, totalBytes, onProgress)
    }

    // Cleanup
    abortController.abort()
    await monitorDone.catch(() => {})
    try {
      monitorClient.destroy()
    } catch {
      /* ignore */
    }
    for (const w of workers) w.destroy()
  }

  // -----------------------------------------------------------------------
  // 5. Collect results
  // -----------------------------------------------------------------------
  const durationMs = Date.now() - startTime
  const elapsedSec = durationMs / 1000

  return {
    totalSubmitted: stats.submitted,
    totalConfirmed: stats.confirmed,
    totalErrors: stats.errors,
    submitted: stats.submitted,
    confirmed: stats.confirmed,
    errors: stats.errors,
    poolFullRetries: stats.poolFullRetries,
    staleNonces: stats.staleNonces,
    elapsedMs: durationMs,
    durationMs,
    totalBytes,
    throughputBytesPerSec: elapsedSec > 0 ? totalBytes / elapsedSec : 0,
    txPerSec: elapsedSec > 0 ? stats.confirmed / elapsedSec : 0,
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Extended PAPI transaction interface with `sign()` method.
 *
 * The `sign` method is available on actual PAPI Transaction objects but
 * is not declared in the SDK's minimal {@link PapiTransaction} interface.
 * We use this type assertion internally for pre-signing.
 */
interface SignableTransaction {
  sign(
    from: PolkadotSigner,
    txOptions?: { nonce?: number },
  ): Promise<string>
}

/** Block until estimated pending ≤ threshold. */
async function waitForBackpressure(
  stats: SharedStats,
  threshold: number,
): Promise<void> {
  for (;;) {
    const pending = stats.submitted - stats.confirmed
    if (pending <= threshold) return
    await stats.waitForBlock(500)
  }
}

/** Emit progress stats via callback. */
function emitProgress(
  stats: SharedStats,
  startTime: number,
  totalBytes: number,
  onProgress: (stats: PipelineStats) => void,
): void {
  const elapsedMs = Date.now() - startTime
  const elapsedSec = elapsedMs / 1000
  onProgress({
    submitted: stats.submitted,
    confirmed: stats.confirmed,
    errors: stats.errors,
    poolFullRetries: stats.poolFullRetries,
    staleNonces: stats.staleNonces,
    elapsedMs,
    throughputBytesPerSec: elapsedSec > 0 ? totalBytes / elapsedSec : 0,
    txPerSec: elapsedSec > 0 ? stats.confirmed / elapsedSec : 0,
  })
}

function emptyResult(): PipelineResult {
  return {
    totalSubmitted: 0,
    totalConfirmed: 0,
    totalErrors: 0,
    submitted: 0,
    confirmed: 0,
    errors: 0,
    poolFullRetries: 0,
    staleNonces: 0,
    elapsedMs: 0,
    durationMs: 0,
    totalBytes: 0,
    throughputBytesPerSec: 0,
    txPerSec: 0,
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/**
 * Derive the SS58 address from the signer.
 *
 * PAPI signers expose `publicKey` which we encode as SS58.
 * We query the chain for the SS58 prefix via `system_properties`.
 */
async function getSignerAddress(
  signer: PolkadotSigner,
  _api: BulletinTypedApi,
): Promise<string> {
  // PolkadotSigner.publicKey is a Uint8Array (32 bytes for sr25519/ed25519).
  // For `system_accountNextIndex`, we can pass the hex-encoded public key
  // prefixed with "0x" — the RPC accepts both SS58 and raw hex.
  const pubKey = signer.publicKey
  return (
    "0x" +
    Array.from(pubKey)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("")
  )
}
