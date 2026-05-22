// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Async client with full transaction submission support
 */

import { ss58Address } from "@polkadot-labs/hdkd-helpers"
import type { CID } from "multiformats/cid"
import { Binary, type PolkadotSigner } from "polkadot-api"
import { getWsProvider } from "polkadot-api/ws"
import {
  isStallError,
  type PipelineBootstrap,
  pipelineStore,
} from "./pipeline.js"
import { BulletinPreparer } from "./preparer.js"
import {
  BulletinError,
  type ChunkerConfig,
  CidCodec,
  type ClientConfig,
  DEFAULT_STORE_OPTIONS,
  ErrorCode,
  HashAlgorithm,
  type ProgressCallback,
  resolveClientConfig,
  type StoreOptions,
  type StoreResult,
  TxStatus,
  type UploadCallback,
  type UploadFileResult,
  type UploadItem,
  type UploadResult,
  UploadStatus,
  type WaitFor,
} from "./types.js"
import {
  calculateCid,
  getContentHash,
  hashAlgorithmCodecToEnum,
  isNonDefaultCidConfig,
  normalizeHex,
  type ScaleHashingAlgorithm,
} from "./utils.js"

/**
 * Minimal interface for a decoded PAPI runtime event.
 *
 * PAPI events from chain metadata have the shape:
 * `{ type: "PalletName", value: { type: "EventName", value: { ...fields } } }`
 */
interface RuntimeEvent {
  type: string
  value?: { type?: string; value?: { index?: number } }
}

/**
 * Minimal interface for PAPI transaction status events
 * (union of TxSigned, TxBroadcasted, TxBestBlocksState, TxFinalized).
 */
interface TxStatusEvent {
  txHash?: string
  type?: string
  found?: boolean
  nPeers?: number
  block?: { hash: string; number: number; index?: number }
  events?: RuntimeEvent[]
}

/**
 * Minimal interface for a PAPI transaction.
 *
 * Describes the subset of PAPI's `Transaction` type that the SDK uses.
 * The actual type is generic over chain descriptors; this interface avoids
 * requiring generated chain types as a dependency.
 */
interface PapiTransaction {
  signAndSubmit(signer: PolkadotSigner): Promise<{
    block?: { hash: string; number: number }
    txHash: string
    events?: RuntimeEvent[]
  }>
  signSubmitAndWatch(signer: PolkadotSigner): {
    subscribe(observer: {
      next: (ev: TxStatusEvent) => void
      error: (err: unknown) => void
      complete?: () => void
    }): { unsubscribe(): void }
  }
  /** SCALE-encoded bare (unsigned) transaction ready for broadcasting */
  getBareTx(): Promise<Uint8Array>
  decodedCall: unknown
}

/**
 * Minimal interface for the PAPI typed API.
 *
 * Describes the pallets and extrinsics the SDK interacts with.
 * Users pass their actual `TypedApi<ChainDescriptor>` which satisfies
 * this interface structurally.
 */
export interface BulletinTypedApi {
  tx: {
    TransactionStorage: {
      store(args: { data: Uint8Array }): PapiTransaction
      store_with_cid_config(args: {
        cid: { codec: bigint; hashing: ScaleHashingAlgorithm }
        data: Uint8Array
      }): PapiTransaction
      authorize_account(args: {
        who: string
        transactions: number
        bytes: bigint
      }): PapiTransaction
      authorize_preimage(args: {
        content_hash: string
        max_size: bigint
      }): PapiTransaction
      renew(args: { block: number; index: number }): PapiTransaction
      remove_expired_account_authorization(args: {
        who: string
      }): PapiTransaction
      remove_expired_preimage_authorization(args: {
        content_hash: string
      }): PapiTransaction
      refresh_account_authorization(args: { who: string }): PapiTransaction
      refresh_preimage_authorization(args: {
        content_hash: string
      }): PapiTransaction
    }
    Sudo?: {
      sudo(args: { call: unknown }): PapiTransaction
    }
  }
  /** Optional query interface for on-chain storage reads (e.g., authorization checks) */
  query?: {
    TransactionStorage: {
      Authorizations: {
        getValue(
          scope: { type: string; value: unknown },
          opts?: { at?: string },
        ): Promise<
          | {
              extent: {
                transactions: number
                /** Newer chains expose the cap separately from consumed counters. */
                transactions_allowance?: number
                bytes: bigint
                /** Newer chains expose the cap separately from consumed counters. */
                bytes_allowance?: bigint
              }
              expiration: number
            }
          | undefined
        >
      }
      /**
       * `H256 → (BlockNumber, ExtrinsicIndex)`. Populated by the pallet's
       * `store` dispatchable at execution. Used by `pipelineStore` to
       * reconcile per-item finalization without trusting nonce arithmetic.
       *
       * PAPI decodes `(BlockNumber, u32)` as a 2-tuple; runtime shape may
       * vary by metadata version (some emit named fields), so callers
       * should consume the value defensively.
       */
      TransactionByContentHash: {
        getValue(contentHash: string, opts?: { at?: string }): Promise<unknown>
      }
    }
  }
}

/**
 * Stream-watch submission interface, matching the signature of
 * `PolkadotClient.submitAndWatch` from polkadot-api. Pass
 * `papiClient.submitAndWatch` when constructing the client.
 *
 * Required only for unsigned (`asUnsigned()`) uploads — signed uploads
 * use the pipelined engine with its own wsUrls-based provider.
 */
export type SubmitAndWatchFn = (transaction: Uint8Array) => {
  subscribe(observer: {
    next: (ev: TxStatusEvent) => void
    error: (err: unknown) => void
    complete?: () => void
  }): { unsubscribe(): void }
}

/**
 * Transaction receipt from a successful submission
 */
export interface TransactionReceipt {
  /** Block hash containing the transaction */
  blockHash: string
  /** Transaction hash */
  txHash: string
  /** Block number (if known) */
  blockNumber?: number
}

/** Options for transaction submission */
export interface CallOptions {
  /** Callback to receive transaction status events */
  onProgress?: ProgressCallback
  /** What to wait for before returning (default: "in_block") */
  waitFor?: WaitFor
}

/** Options for authorization calls that may require sudo */
export interface AuthCallOptions extends CallOptions {
  /** Wrap the call in Sudo (for chains where Authorizer origin requires it) */
  sudo?: boolean
}

/**
 * Transaction status extracted from a PAPI event by `mapPapiEventToProgress`.
 *
 * `txHash` - set when the event carries a new transaction hash.
 * `finish` - set when the transaction reached the desired confirmation level.
 */
interface MappedTxStatus {
  txHash?: string
  finish?: {
    block: { hash: string; number: number }
    events?: RuntimeEvent[]
  }
}

/**
 * Map a raw PAPI transaction status event to SDK progress events.
 *
 * Extracted from `signAndSubmitWithProgress` so the event→progress
 * translation is testable independently.  The caller is responsible for
 * acting on the returned `finish` signal.
 */
function mapPapiEventToProgress(
  ev: TxStatusEvent,
  currentTxHash: string | undefined,
  progressCallback: ProgressCallback | undefined,
  chunkIndex: number | undefined,
  waitFor: "in_block" | "finalized" = "finalized",
): MappedTxStatus {
  const result: MappedTxStatus = {}

  // Capture the transaction hash on the first event that carries it
  if (ev.txHash && !currentTxHash) {
    result.txHash = ev.txHash as string
    progressCallback?.({
      type: TxStatus.Signed,
      txHash: result.txHash,
      chunkIndex,
    })
  }

  if (ev.type === "validated") {
    progressCallback?.({ type: TxStatus.Validated, chunkIndex })
  }

  if (ev.type === "broadcasted") {
    progressCallback?.({
      type: TxStatus.Broadcasted,
      chunkIndex,
    })
  }

  if (ev.type === "txBestBlocksState") {
    if (ev.found && ev.block) {
      progressCallback?.({
        type: TxStatus.InBlock,
        blockHash: ev.block.hash,
        blockNumber: ev.block.number,
        txIndex: ev.block.index,
        chunkIndex,
      })
      if (waitFor === "in_block") {
        result.finish = { block: ev.block, events: ev.events }
      }
    } else {
      progressCallback?.({ type: TxStatus.NoLongerInBlock, chunkIndex })
    }
  }

  if (ev.type === "finalized" && ev.block) {
    progressCallback?.({
      type: TxStatus.Finalized,
      blockHash: ev.block.hash,
      blockNumber: ev.block.number,
      txIndex: ev.block.index,
      chunkIndex,
    })
    result.finish = { block: ev.block, events: ev.events }
  }

  return result
}

/**
 * Shared interface for Bulletin clients (real and mock).
 *
 * Both `AsyncBulletinClient` and `MockBulletinClient` implement this interface.
 */
export interface BulletinClientInterface {
  uploadFile(data: Uint8Array): UploadFileBuilder
  upload(items: UploadItem[]): UploadBuilder
  authorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
  ): AuthCallBuilder
  authorizePreimage(contentHash: Uint8Array, maxSize: bigint): AuthCallBuilder
  renew(block: number, index: number): CallBuilder
  refreshAccountAuthorization(who: string): AuthCallBuilder
  refreshPreimageAuthorization(contentHash: Uint8Array): AuthCallBuilder
  removeExpiredAccountAuthorization(who: string): CallBuilder
  removeExpiredPreimageAuthorization(contentHash: Uint8Array): CallBuilder
  estimateAuthorization(dataSize: number): {
    transactions: number
    bytes: number
  }
  /** Release resources held on behalf of this client (e.g. underlying PAPI client). */
  destroy(): Promise<void>
}

/** Dispatch callback for the low-level `upload(items)` execution path. */
type UploadDispatch = (
  items: UploadItem[],
  waitFor: WaitFor,
  onEvent: UploadCallback | undefined,
  checkAuth: boolean,
  unsigned: boolean,
) => Promise<UploadResult>

/** Dispatch callback for the high-level `uploadFile(data)` execution path. */
type UploadFileDispatch = (
  data: Uint8Array,
  waitFor: WaitFor,
  onEvent: UploadCallback | undefined,
  chunkerConfig: Partial<ChunkerConfig> | undefined,
  checkAuth: boolean,
  unsigned: boolean,
) => Promise<UploadFileResult>

/**
 * Shared base for upload builders. Holds the fluent options every upload
 * path supports — callback, wait-for, opt-in authorization pre-flight —
 * and exposes them as `withCallback` / `withWaitFor` / `ensureAuthorized`.
 *
 * Pre-flight: bulletin's `AllowanceBasedPriority` lowers priority on
 * exhausted allowance but doesn't reject, so `ensureAuthorized()` only
 * verifies that an `Authorizations` entry exists and isn't expired. Throws
 * `BulletinError(INSUFFICIENT_AUTHORIZATION)` so the caller can authorize
 * and retry.
 */
abstract class BaseUploadBuilder<TResult> {
  protected callback?: UploadCallback
  protected waitFor: WaitFor = "finalized"
  protected checkAuth = false
  protected unsigned = false

  withCallback(callback: UploadCallback): this {
    this.callback = callback
    return this
  }

  withWaitFor(waitFor: WaitFor): this {
    this.waitFor = waitFor
    return this
  }

  ensureAuthorized(): this {
    this.checkAuth = true
    return this
  }

  /**
   * Submit as unsigned, preimage-authorized extrinsic(s). Requires each
   * item's `blake2b256(data)` to be preimage-authorized on-chain
   * beforehand (typically via `authorizePreimage()`).
   *
   * On `UploadBuilder`, all items are submitted in parallel — each is
   * its own independent unsigned tx that lands when the pool accepts it.
   * On `UploadFileBuilder`, only single-tx uploads are allowed (the
   * chunking + DAG-PB manifest pipeline doesn't support unsigned); throws
   * if `data` exceeds the chunking threshold.
   *
   * Progress events (ItemStarted/InBlock/Finalized/Failed) fire per
   * item with `index` matching its position in the input.
   *
   * When combined with `ensureAuthorized()`, the pre-flight checks each
   * item's `Authorizations<Preimage(blake2b256(data))>` entry instead of
   * the signer's account authorization. Duplicate content hashes across
   * items are deduped before the RPC queries.
   */
  asUnsigned(): this {
    this.unsigned = true
    return this
  }

  abstract send(): Promise<TResult>
}

/**
 * Builder for the low-level `upload(items)` API. Each item becomes one
 * `store` extrinsic; resolves with the per-item CIDs (positional, input order).
 */
export class UploadBuilder extends BaseUploadBuilder<UploadResult> {
  constructor(
    private dispatch: UploadDispatch,
    private items: UploadItem[],
  ) {
    super()
  }

  async send(): Promise<UploadResult> {
    return this.dispatch(
      this.items,
      this.waitFor,
      this.callback,
      this.checkAuth,
      this.unsigned,
    )
  }
}

/**
 * Builder for the high-level `uploadFile(data)` API. Auto-chunks the data,
 * builds a DAG-PB manifest, and submits everything through the same
 * pipeline. Resolves with the single root CID.
 *
 * @example
 * ```typescript
 * const { cid } = await client
 *   .uploadFile(bytes)
 *   .withCallback((event) => console.log(event))
 *   .send();
 * ```
 */
export class UploadFileBuilder extends BaseUploadBuilder<UploadFileResult> {
  private chunkerConfig?: Partial<ChunkerConfig>

  constructor(
    private dispatch: UploadFileDispatch,
    private data: Uint8Array,
  ) {
    super()
  }

  /** Override the chunk size (forces chunked upload path even for small files). */
  withChunkSize(chunkSize: number): this {
    this.chunkerConfig = { ...this.chunkerConfig, chunkSize }
    return this
  }

  /** Disable manifest creation. Without a manifest, only the last chunk CID is returned. */
  withManifest(enabled: boolean): this {
    this.chunkerConfig = { ...this.chunkerConfig, createManifest: enabled }
    return this
  }

  async send(): Promise<UploadFileResult> {
    return this.dispatch(
      this.data,
      this.waitFor,
      this.callback,
      this.chunkerConfig,
      this.checkAuth,
      this.unsigned,
    )
  }
}

/**
 * Builder for calls with `CallOptions` (waitFor + callback)
 *
 * Used by: `renew`, `removeExpiredAccountAuthorization`, `removeExpiredPreimageAuthorization`
 *
 * @example
 * ```typescript
 * const receipt = await client
 *   .renew(blockNumber, index)
 *   .withWaitFor('finalized')
 *   .withCallback((event) => console.log(event))
 *   .send();
 * ```
 */
export class CallBuilder {
  private options: CallOptions = {}
  constructor(
    private executor: (options: CallOptions) => Promise<TransactionReceipt>,
  ) {}
  /** Set what to wait for before returning */
  withWaitFor(waitFor: WaitFor): this {
    this.options.waitFor = waitFor
    return this
  }
  /** Set progress callback */
  withCallback(callback: ProgressCallback): this {
    this.options.onProgress = callback
    return this
  }
  /** Submit the transaction */
  async send(): Promise<TransactionReceipt> {
    return this.executor(this.options)
  }
}

/**
 * Builder for authorization calls that may require sudo
 *
 * Used by: `authorizeAccount`, `authorizePreimage`, `refreshAccountAuthorization`, `refreshPreimageAuthorization`
 *
 * @example
 * ```typescript
 * const receipt = await client
 *   .authorizeAccount(who, transactions, bytes)
 *   .withSudo()
 *   .withCallback((event) => console.log(event))
 *   .send();
 * ```
 */
export class AuthCallBuilder {
  private options: AuthCallOptions = {}
  constructor(
    private executor: (options: AuthCallOptions) => Promise<TransactionReceipt>,
  ) {}
  /** Set what to wait for before returning */
  withWaitFor(waitFor: WaitFor): this {
    this.options.waitFor = waitFor
    return this
  }
  /** Set progress callback */
  withCallback(callback: ProgressCallback): this {
    this.options.onProgress = callback
    return this
  }
  /** Wrap the call in Sudo */
  withSudo(): this {
    this.options.sudo = true
    return this
  }
  /** Submit the transaction */
  async send(): Promise<TransactionReceipt> {
    return this.executor(this.options)
  }
}

/** Resolve store options with defaults */
function resolveStoreOptions(options?: StoreOptions): {
  cidCodec: CidCodec | number
  hashAlgorithm: HashAlgorithm
  waitFor: WaitFor
} {
  const opts = { ...DEFAULT_STORE_OPTIONS, ...options }
  return {
    cidCodec: opts.cidCodec ?? CidCodec.Raw,
    hashAlgorithm: opts.hashingAlgorithm ?? HashAlgorithm.Blake2b256,
    waitFor: opts.waitFor ?? "in_block",
  }
}

/**
 * Reject uploads whose items have duplicate content hashes. The pipeline's
 * `TransactionByContentHash`-based reconciler identifies items by their
 * content hash; two items with the same hash would map to one TBCH entry,
 * making per-item finalization undecidable. Catch this at submission time
 * with a clear error rather than silently stalling.
 *
 * Returns the index of the first duplicate so the caller can act on it.
 */
function assertUniqueContentHashes(cids: CID[]): void {
  const seen = new Map<string, number>()
  for (let i = 0; i < cids.length; i++) {
    const hex = normalizeHex(Binary.toHex(cids[i]!.multihash.digest))
    const prior = seen.get(hex)
    if (prior !== undefined) {
      throw new BulletinError(
        `upload(): item ${i} has the same content hash as item ${prior} — the SDK identifies items by content hash and can't distinguish duplicates. If you need to store the same data multiple times, submit them in separate upload() calls.`,
        ErrorCode.INVALID_CONFIG,
      )
    }
    seen.set(hex, i)
  }
}

/**
 * Extract the `Stored.index` for a specific content hash from a block's
 * runtime events.
 *
 * `contentHashHex` is the blake2b-256 hash of the data, hex-encoded (with
 * or without leading `0x`). If omitted, returns the first Stored event's
 * index — useful when the caller already knows there's exactly one match.
 */
function extractStoredIndex(
  events?: RuntimeEvent[],
  contentHashHex?: string,
): number | undefined {
  if (!events) return undefined
  const target = contentHashHex ? normalizeHex(contentHashHex) : undefined
  for (const e of events) {
    if (e.type !== "TransactionStorage" || e.value?.type !== "Stored") continue
    if (target) {
      const ch = (e.value?.value as { content_hash?: unknown } | undefined)
        ?.content_hash
      const chHex =
        typeof ch === "string"
          ? normalizeHex(ch)
          : ch instanceof Uint8Array
            ? normalizeHex(Binary.toHex(ch))
            : undefined
      if (chHex !== target) continue
    }
    return (e.value?.value as { index?: number } | undefined)?.index
  }
  return undefined
}

/**
 * Async Bulletin client that submits transactions to the chain
 *
 * This client is tightly coupled to PAPI (Polkadot API) for blockchain interaction.
 * Users must provide a configured PAPI client with appropriate chain metadata.
 *
 * @example
 * ```typescript
 * import { createClient } from 'polkadot-api';
 * import { getWsProvider } from 'polkadot-api/ws';
 * import { AsyncBulletinClient } from '@parity/bulletin-sdk';
 *
 * // User sets up PAPI client
 * const wsProvider = getWsProvider('wss://bulletin-rpc.polkadot.io');
 * const client = createClient(wsProvider);
 * const api = client.getTypedApi(bulletinDescriptor);
 *
 * // Create SDK client
 * const bulletinClient = new AsyncBulletinClient(api, signer, papiClient.submitAndWatch);
 *
 * // Store data
 * const result = await bulletinClient.store(data).send();
 * ```
 */
export class AsyncBulletinClient implements BulletinClientInterface {
  /** PAPI client for blockchain interaction */
  public api: BulletinTypedApi
  /**
   * Signer for transaction signing. May be `undefined` when the client is
   * constructed for unsigned-only use (`asUnsigned()`); any signed code
   * path will throw if invoked.
   */
  public signer: PolkadotSigner | undefined
  /**
   * Stream-watch submission (`PolkadotClient.submitAndWatch`). Required
   * for unsigned (`asUnsigned()`) uploads — signed uploads use the
   * pipelined engine and don't need it, so it can be `undefined` on a
   * signed-only client.
   */
  public submitAndWatch: SubmitAndWatchFn | undefined
  /** Client configuration */
  public config: Required<ClientConfig>
  /** Offline operations (chunking, CID calculation, estimation) */
  private preparer: BulletinPreparer
  /** Optional teardown callback invoked by `destroy()` */
  private onDestroy?: () => void | Promise<void>
  /**
   * Mutable pipeline bootstrap cache shared across every upload from this
   * client. Populated on the first successful `pipelineStore` call (metadata
   * fetch + offline-API build); subsequent calls skip the round-trip. Survives
   * the lifetime of the client.
   */
  private pipelineBootstrap: PipelineBootstrap = {}

  /**
   * Create a new async client with PAPI client and signer
   *
   * The PAPI client must be configured with the correct chain metadata
   * for your Bulletin Chain node.
   *
   * @param api - Configured PAPI TypedApi instance
   * @param signer - Polkadot signer for transaction signing, or `undefined`
   *   for an unsigned-only client (`asUnsigned()` paths). Signed paths
   *   throw `UNSUPPORTED_OPERATION` when invoked on a signer-less client.
   * @param submitAndWatch - Stream-watch submission (pass
   *   `papiClient.submitAndWatch`). Required for unsigned (`asUnsigned()`)
   *   uploads. Pass `undefined` if you only use signed uploads — the
   *   signed engine uses its own wsUrls-based provider.
   * @param config - Optional client configuration
   * @param onDestroy - Optional teardown callback. When provided, `destroy()`
   *   awaits it so callers (e.g. wrappers that own the underlying
   *   `PolkadotClient`) can route cleanup through this client.
   */
  constructor(
    api: BulletinTypedApi,
    signer: PolkadotSigner | undefined,
    submitAndWatch: SubmitAndWatchFn | undefined,
    config?: Partial<ClientConfig>,
    onDestroy?: () => void | Promise<void>,
  ) {
    this.api = api
    this.signer = signer
    this.submitAndWatch = submitAndWatch
    this.config = resolveClientConfig(config)
    this.onDestroy = onDestroy
    this.preparer = new BulletinPreparer({
      defaultChunkSize: this.config.defaultChunkSize,
      createManifest: this.config.createManifest,
      chunkingThreshold: this.config.chunkingThreshold,
    })
  }

  /**
   * Release resources held on behalf of this client.
   *
   * Invokes the optional `onDestroy` callback supplied at construction time.
   * Without one, this is a no-op — the SDK itself holds no long-lived
   * resources, so callers that own the underlying `PolkadotClient` (or other
   * connection) can either tear it down themselves or pass `onDestroy` to
   * route teardown through here.
   */
  async destroy(): Promise<void> {
    await this.onDestroy?.()
  }

  /**
   * Opt-in pre-flight (via the builder's `ensureAuthorized()`): verify that
   * the signer has a non-expired `Authorizations` entry on chain. The chain
   * does not reject store calls when allowance is exhausted — it lowers
   * priority via `AllowanceBasedPriority` — so existence + expiry is the
   * only useful client-side check. Throws
   * `BulletinError(INSUFFICIENT_AUTHORIZATION)` on failure so the caller
   * can authorize and retry.
   */
  private async ensureAuthorizedOnChain(): Promise<void> {
    // If the typed API doesn't expose query at all, we can't honor the
    // caller's fail-fast opt-in — surface that instead of silently passing.
    if (!this.api.query) {
      throw new BulletinError(
        "ensureAuthorized(): the typed API does not expose query support; cannot verify authorization",
        ErrorCode.UNSUPPORTED_OPERATION,
      )
    }
    this.requireSigner("ensureAuthorized()")
    const address = ss58Address(this.signer!.publicKey)
    const auth =
      await this.api.query.TransactionStorage.Authorizations.getValue({
        type: "Account",
        value: address,
      })
    if (!auth) {
      throw new BulletinError(
        `Account ${address} is not authorized to store data on this chain`,
        ErrorCode.INSUFFICIENT_AUTHORIZATION,
      )
    }
    // Compare expiration (block number) against the current best block.
    // If api.query.System.Number isn't exposed we skip — the chain rejects
    // expired auths at submission time anyway.
    const sysNumber = (
      this.api.query as {
        System?: { Number?: { getValue(): Promise<number> } }
      }
    ).System?.Number
    if (sysNumber && typeof auth.expiration === "number") {
      const currentBlock = await sysNumber.getValue()
      if (auth.expiration <= currentBlock) {
        throw new BulletinError(
          `Authorization for ${address} expired at block ${auth.expiration} (current ${currentBlock})`,
          ErrorCode.INSUFFICIENT_AUTHORIZATION,
        )
      }
    }
  }

  /**
   * Opt-in pre-flight for the unsigned (`asUnsigned()`) path: verify that
   * every item has a non-expired `Authorizations<Preimage(blake2b256(data))>`
   * entry on chain. Preimage authorization is what the runtime checks for
   * an unsigned `store` extrinsic — without it the tx is rejected by
   * `AuthorizeCall`. Throws `INSUFFICIENT_AUTHORIZATION` for the first
   * missing/expired item so the caller can authorize and retry.
   */
  private async ensurePreimagesAuthorized(
    items: UploadItem[],
    cids?: CID[],
  ): Promise<void> {
    const query = this.api.query
    if (!query) {
      throw new BulletinError(
        "ensureAuthorized(): the typed API does not expose query support; cannot verify authorization",
        ErrorCode.UNSUPPORTED_OPERATION,
      )
    }
    const sysNumber = (
      query as { System?: { Number?: { getValue(): Promise<number> } } }
    ).System?.Number
    const currentBlock = sysNumber ? await sysNumber.getValue() : undefined

    // The pallet keys preimage authorizations by the user's chosen hash
    // algo's digest, which is exactly the CID's `multihash.digest`. Reuse
    // pre-computed CIDs when the caller has them; otherwise hash now.
    const hashHexes = cids
      ? cids.map((cid) => Binary.toHex(cid.multihash.digest))
      : await Promise.all(
          items.map(async (item) => {
            const algo = item.hashAlgo ?? HashAlgorithm.Blake2b256
            return Binary.toHex(await getContentHash(item.data, algo))
          }),
        )
    const uniqueHashes = Array.from(new Set(hashHexes))

    await Promise.all(
      uniqueHashes.map(async (hashHex) => {
        const auth = await query.TransactionStorage.Authorizations.getValue({
          type: "Preimage",
          value: hashHex,
        })
        if (!auth) {
          throw new BulletinError(
            `No preimage authorization on chain for content hash ${hashHex}`,
            ErrorCode.INSUFFICIENT_AUTHORIZATION,
          )
        }
        if (
          currentBlock !== undefined &&
          typeof auth.expiration === "number" &&
          auth.expiration <= currentBlock
        ) {
          throw new BulletinError(
            `Preimage authorization for ${hashHex} expired at block ${auth.expiration} (current ${currentBlock})`,
            ErrorCode.INSUFFICIENT_AUTHORIZATION,
          )
        }
      }),
    )
  }

  /**
   * Assert a signer is wired before invoking a signed code path. Clients
   * constructed for unsigned-only use (`asUnsigned()`) pass `undefined`
   * for `signer`; calling a signed method on them raises here with a
   * clear error rather than crashing in the depths of PAPI.
   */
  private requireSigner(operation: string): void {
    if (!this.signer) {
      throw new BulletinError(
        `${operation} requires a signer, but this client was constructed without one`,
        ErrorCode.UNSUPPORTED_OPERATION,
      )
    }
  }

  /**
   * Create a store transaction.
   *
   * The chain defaults to Raw (0x55) codec + Blake2b-256 hashing, so the plain
   * `store()` extrinsic is sufficient for the common case. We only use the heavier
   * `store_with_cid_config()` extrinsic when the user requests non-default settings.
   */
  private createStoreTx(
    data: Uint8Array,
    cidCodec: CidCodec | number,
    hashAlgorithm: HashAlgorithm,
  ): PapiTransaction {
    return isNonDefaultCidConfig(cidCodec, hashAlgorithm)
      ? this.api.tx.TransactionStorage.store_with_cid_config({
          cid: {
            codec: BigInt(cidCodec),
            hashing: hashAlgorithmCodecToEnum(hashAlgorithm),
          },
          data,
        })
      : this.api.tx.TransactionStorage.store({ data })
  }

  /**
   * Sign, submit, and watch a transaction with progress callbacks.
   *
   * Uses PAPI's signSubmitAndWatch which provides real-time status updates
   * as the transaction progresses through the network.
   *
   * @param tx - The transaction to submit
   * @param progressCallback - Optional callback to receive transaction status events
   * @param waitFor - What to wait for: "in_block" (faster) or "finalized" (safer, default)
   */
  private async signAndSubmitWithProgress(
    tx: PapiTransaction,
    progressCallback?: ProgressCallback,
    waitFor: "in_block" | "finalized" = "finalized",
    chunkIndex?: number,
  ): Promise<{
    blockHash: string
    txHash: string
    blockNumber?: number
    txIndex?: number
    events?: RuntimeEvent[]
  }> {
    this.requireSigner("signAndSubmitWithProgress")
    return new Promise((resolve, reject) => {
      let resolved = false
      let txHash: string | undefined

      const cleanup = () => {
        clearTimeout(timerId)
        subscription.unsubscribe()
      }

      const finish = (
        block: { hash: string; number: number },
        events?: RuntimeEvent[],
      ) => {
        if (resolved) return
        resolved = true
        cleanup()
        resolve({
          blockHash: block.hash,
          txHash: txHash || "",
          blockNumber: block.number,
          txIndex: extractStoredIndex(events),
          events,
        })
      }

      const subscription = tx.signSubmitAndWatch(this.signer!).subscribe({
        next: (ev: TxStatusEvent) => {
          const result = mapPapiEventToProgress(
            ev,
            txHash,
            progressCallback,
            chunkIndex,
            waitFor,
          )
          if (result.txHash) txHash = result.txHash
          if (result.finish) finish(result.finish.block, result.finish.events)
        },
        error: (err: unknown) => {
          if (!resolved) {
            resolved = true
            cleanup()
            if (progressCallback) {
              const errorMsg = err instanceof Error ? err.message : String(err)
              // Distinguish pool-related drops from other transaction errors
              const isDropped =
                errorMsg.includes("dropped") || errorMsg.includes("pool")
              progressCallback({
                type: isDropped ? TxStatus.Dropped : TxStatus.Invalid,
                error: errorMsg,
                chunkIndex,
              })
            }
            reject(err)
          }
        },
        complete: () => {
          // PAPI can complete the Observable without a finalized/in_block
          // event (e.g. txBestBlocksState fires with found:false after a
          // reorg or node restart, causing the internal continueWith() to
          // map to rxjs.EMPTY which completes immediately). Without this
          // handler the Promise hangs until the defensive timeout fires.
          if (!resolved) {
            resolved = true
            cleanup()
            progressCallback?.({
              type: TxStatus.Dropped,
              error:
                "Transaction subscription ended before reaching the expected status",
              chunkIndex,
            })
            reject(
              new BulletinError(
                "Transaction subscription ended before reaching the expected status. " +
                  "This usually means the transaction was dropped from the best block " +
                  "(e.g. due to a chain reorganization or node restart).",
                ErrorCode.TRANSACTION_FAILED,
              ),
            )
          }
        },
      })

      // Defensive timeout: PAPI handles reconnects and mortality, so this
      // should rarely fire. If it does, it likely indicates a bug. Default:
      // 7 min (above PAPI's 64-block mortality window).
      const timerId = setTimeout(() => {
        if (resolved) return
        resolved = true
        cleanup()
        reject(new BulletinError("Transaction timed out", ErrorCode.TIMEOUT))
      }, this.config.txTimeout)
    })
  }

  /**
   * Wrap a call in Sudo if requested, otherwise return it as-is
   */
  private maybeSudo(tx: PapiTransaction, sudo?: boolean): PapiTransaction {
    if (!sudo) return tx
    if (!this.api.tx.Sudo) {
      throw new BulletinError(
        "sudo requested but Sudo pallet is not available on this chain",
        ErrorCode.INVALID_CONFIG,
      )
    }
    return this.api.tx.Sudo.sudo({ call: tx.decodedCall })
  }

  /**
   * Submit a transaction, returning a receipt on success or throwing a BulletinError on failure.
   */
  private async submitTx(
    tx: PapiTransaction,
    errorMessage: string,
    errorCode: ErrorCode,
    options?: CallOptions,
  ): Promise<TransactionReceipt> {
    try {
      const waitFor = options?.waitFor ?? "in_block"
      const result = await this.signAndSubmitWithProgress(
        tx,
        options?.onProgress,
        waitFor,
      )

      return {
        blockHash: result.blockHash,
        txHash: result.txHash,
        blockNumber: result.blockNumber,
      }
    } catch (error) {
      if (error instanceof BulletinError) throw error
      throw new BulletinError(`${errorMessage}: ${error}`, errorCode, error)
    }
  }

  /**
   * High-level upload: chunk (if needed), build a DAG-PB manifest, and submit
   * everything through the shared submission pipeline. Returns the single
   * root CID the caller can use to retrieve the file later.
   *
   * **Memory.** `uploadFile` retains the full `data` plus every chunk and
   * every per-wave signed hex string in RAM until the promise resolves. For
   * a 100 MiB file expect peak RSS of roughly 300 MiB (data + chunks + hex
   * inflation during broadcast). For larger files or memory-constrained
   * environments, use {@link upload} with caller-managed chunks so older
   * buffers can be freed once their `ItemFinalized` event has fired.
   *
   * @example
   * ```typescript
   * const { cid } = await client
   *   .uploadFile(bytes)
   *   .withCallback((event) => console.log(event))
   *   .send();
   * ```
   */
  uploadFile(data: Uint8Array): UploadFileBuilder {
    return new UploadFileBuilder(
      (d, wf, oe, cc, ca, un) => this.uploadFileImpl(d, wf, oe, cc, ca, un),
      data,
    )
  }

  /**
   * Low-level upload: submit a list of items as one `store` extrinsic each.
   *
   * Each item is signed and broadcast through the shared submission pipeline,
   * regardless of how many items are passed. Per-item CIDs are computed by
   * the SDK from `(data, codec, hashAlgo)` and surface in every progress
   * event. Returns the CIDs positionally, matching input order.
   */
  upload(items: UploadItem[]): UploadBuilder {
    return new UploadBuilder(
      (its, wf, oe, ca, un) => this.uploadItemsImpl(its, wf, oe, ca, un),
      items,
    )
  }

  private async uploadFileImpl(
    data: Uint8Array,
    waitFor: WaitFor,
    onEvent: UploadCallback | undefined,
    chunkerConfig: Partial<ChunkerConfig> | undefined,
    checkAuth: boolean,
    unsigned: boolean,
  ): Promise<UploadFileResult> {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", ErrorCode.EMPTY_DATA)
    }
    const shouldChunk =
      !!chunkerConfig || data.length > this.config.chunkingThreshold
    if (unsigned && shouldChunk) {
      throw new BulletinError(
        "asUnsigned() does not support chunked uploads (data exceeds chunkingThreshold or chunker config was provided)",
        ErrorCode.UNSUPPORTED_OPERATION,
      )
    }
    if (!shouldChunk) {
      const { cids } = await this.uploadItemsImpl(
        [{ data }],
        waitFor,
        onEvent,
        checkAuth,
        unsigned,
      )
      return { cid: cids[0]! }
    }
    const prepared = await this.preparer.prepareStoreChunked(
      data,
      chunkerConfig,
    )
    const items: UploadItem[] = prepared.chunks.map((c) => ({ data: c.data }))
    if (prepared.manifest) {
      items.push({
        data: prepared.manifest.data,
        codec: CidCodec.DagPb,
      })
    }
    const { cids } = await this.uploadItemsImpl(
      items,
      waitFor,
      onEvent,
      checkAuth,
      false,
    )
    // For chunked uploads the manifest is the last item; without one, the
    // last chunk's CID is the best identifier we have.
    return { cid: cids[cids.length - 1]! }
  }

  private async uploadItemsImpl(
    items: UploadItem[],
    waitFor: WaitFor,
    onEvent: UploadCallback | undefined,
    checkAuth: boolean,
    unsigned: boolean,
  ): Promise<UploadResult> {
    if (items.length === 0) {
      throw new BulletinError(
        "upload() requires at least one item",
        ErrorCode.EMPTY_DATA,
      )
    }
    if (unsigned) {
      return this.uploadUnsignedMany(items, waitFor, onEvent, checkAuth)
    }
    this.requireSigner("upload()")
    if (checkAuth) await this.ensureAuthorizedOnChain()

    // Compute per-item CIDs once across all retry attempts.
    const allItemCids: CID[] = await Promise.all(
      items.map((item) =>
        calculateCid(
          item.data,
          item.codec ?? CidCodec.Raw,
          item.hashAlgo ?? HashAlgorithm.Blake2b256,
        ),
      ),
    )
    assertUniqueContentHashes(allItemCids)
    // Shared bootstrap cache (one per client instance, see field decl)
    // is reused across uploads AND across retry attempts within one upload.
    const bootstrap = this.pipelineBootstrap

    // Retry on transient stalls; resume from finalized count so items that
    // already landed are not re-submitted. CIDs are deterministic from
    // input — `pipelineStore` always returns them computed from the same
    // (data, codec, hashAlgo) we supplied via `precomputedCids`, so we
    // can return `allItemCids` directly instead of stitching across
    // retry attempts (which would lose pre-stall finalized slots).
    const maxRetries = 3
    let attempt = 0
    let alreadyFinalized = 0
    while (true) {
      const remaining = items.slice(alreadyFinalized)
      const remainingCids = allItemCids.slice(alreadyFinalized)
      try {
        await pipelineStore(this.api, this.signer!, remaining, {
          wsUrls: this.config.wsUrls,
          createProvider: (url) => getWsProvider(url),
          blockLimits: this.config.blockLimits,
          completeOn: waitFor === "in_block" ? "best" : "finalized",
          bootstrap,
          precomputedCids: remainingCids,
          onEvent: onEvent
            ? (ev) => onEvent({ ...ev, index: alreadyFinalized + ev.index })
            : undefined,
        })
        break
      } catch (error) {
        if (isStallError(error) && attempt < maxRetries) {
          alreadyFinalized += error.cause.finalized
          attempt += 1
          await new Promise((r) => setTimeout(r, 1_000 * 2 ** (attempt - 1)))
          continue
        }
        if (error instanceof BulletinError) throw error
        throw new BulletinError(
          `upload failed: ${error instanceof Error ? error.message : String(error)}`,
          ErrorCode.TRANSACTION_FAILED,
          error,
        )
      }
    }
    return { cids: allItemCids }
  }

  /**
   * Submit N unsigned (preimage-authorized) store extrinsics through the
   * same pipelined engine as signed uploads. Broadcasts each item once
   * via `author_submitExtrinsic` (no per-tx subscription) and reconciles
   * via a single shared `chainHead_v1_follow` subscription with the
   * TBCH-based reconciler. Eliminates the per-tx `submitAndWatch`
   * subscription cap (~16–64) — N items scale to 1 subscription.
   *
   * Per-item events fire through the shared callback. The `index` is
   * the item's position in the input array.
   */
  private async uploadUnsignedMany(
    items: UploadItem[],
    waitFor: WaitFor,
    onEvent: UploadCallback | undefined,
    checkAuth: boolean,
  ): Promise<UploadResult> {
    for (const item of items) {
      if (item.data.length === 0) {
        throw new BulletinError("Data cannot be empty", ErrorCode.EMPTY_DATA)
      }
    }
    if (this.config.wsUrls.length === 0) {
      throw new BulletinError(
        "asUnsigned() multi-item upload requires the client to be constructed with `wsUrls` (chainHead-based reconciliation)",
        ErrorCode.UNSUPPORTED_OPERATION,
      )
    }

    // Pre-compute CIDs (one per item) — pipelineStore consumes a
    // resolved CIDs array via the precomputedCids hook. Doing this BEFORE
    // ensurePreimagesAuthorized lets that helper reuse the multihash
    // digests as content hashes (avoids re-hashing the data).
    const allItemCids: CID[] = await Promise.all(
      items.map((item) =>
        calculateCid(
          item.data,
          item.codec ?? CidCodec.Raw,
          item.hashAlgo ?? HashAlgorithm.Blake2b256,
        ),
      ),
    )
    assertUniqueContentHashes(allItemCids)
    if (checkAuth) await this.ensurePreimagesAuthorized(items, allItemCids)

    try {
      const result = await pipelineStore(this.api, undefined, items, {
        wsUrls: this.config.wsUrls,
        createProvider: (url) => getWsProvider(url),
        blockLimits: this.config.blockLimits,
        completeOn: waitFor === "in_block" ? "best" : "finalized",
        bootstrap: this.pipelineBootstrap,
        precomputedCids: allItemCids,
        onEvent,
      })
      return { cids: result.cids }
    } catch (error) {
      if (error instanceof BulletinError) throw error
      throw new BulletinError(
        `unsigned upload failed: ${error instanceof Error ? error.message : String(error)}`,
        ErrorCode.TRANSACTION_FAILED,
        error,
      )
    }
  }

  /**
   * Authorize an account to store data
   *
   * @param who - Account address to authorize
   * @param transactions - Number of transactions to authorize
   * @param bytes - Maximum bytes to authorize
   */
  authorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
  ): AuthCallBuilder {
    return new AuthCallBuilder((options) => {
      const authTx = this.api.tx.TransactionStorage.authorize_account({
        who,
        transactions,
        bytes,
      })
      return this.submitTx(
        this.maybeSudo(authTx, options?.sudo),
        "Failed to authorize account",
        ErrorCode.AUTHORIZATION_FAILED,
        options,
      )
    })
  }

  /**
   * Authorize a preimage (by content hash) to be stored
   *
   * @param contentHash - Blake2b-256 hash of the content to authorize
   * @param maxSize - Maximum size in bytes for the content
   */
  authorizePreimage(contentHash: Uint8Array, maxSize: bigint): AuthCallBuilder {
    return new AuthCallBuilder((options) => {
      const authTx = this.api.tx.TransactionStorage.authorize_preimage({
        content_hash: Binary.toHex(contentHash),
        max_size: maxSize,
      })
      return this.submitTx(
        this.maybeSudo(authTx, options?.sudo),
        "Failed to authorize preimage",
        ErrorCode.AUTHORIZATION_FAILED,
        options,
      )
    })
  }

  /**
   * Renew/extend retention period for stored data
   *
   * @param block - Block number where the original storage transaction was included
   * @param index - Extrinsic index within the block
   */
  renew(block: number, index: number): CallBuilder {
    return new CallBuilder((options) => {
      const tx = this.api.tx.TransactionStorage.renew({ block, index })
      return this.submitTx(
        tx,
        "Failed to renew",
        ErrorCode.TRANSACTION_FAILED,
        options,
      )
    })
  }

  /**
   * Refresh an account authorization (extends expiry)
   *
   * Requires Authorizer origin on-chain.
   *
   * @param who - Account address to refresh authorization for
   */
  refreshAccountAuthorization(who: string): AuthCallBuilder {
    return new AuthCallBuilder((options) => {
      const authTx =
        this.api.tx.TransactionStorage.refresh_account_authorization({ who })
      return this.submitTx(
        this.maybeSudo(authTx, options?.sudo),
        "Failed to refresh account authorization",
        ErrorCode.AUTHORIZATION_FAILED,
        options,
      )
    })
  }

  /**
   * Refresh a preimage authorization (extends expiry)
   *
   * Requires Authorizer origin on-chain.
   *
   * @param contentHash - Blake2b-256 hash of the authorized content
   */
  refreshPreimageAuthorization(contentHash: Uint8Array): AuthCallBuilder {
    return new AuthCallBuilder((options) => {
      const authTx =
        this.api.tx.TransactionStorage.refresh_preimage_authorization({
          content_hash: Binary.toHex(contentHash),
        })
      return this.submitTx(
        this.maybeSudo(authTx, options?.sudo),
        "Failed to refresh preimage authorization",
        ErrorCode.AUTHORIZATION_FAILED,
        options,
      )
    })
  }

  /**
   * Remove an expired account authorization
   *
   * Can be called by anyone (no special origin required).
   *
   * @param who - Account address with expired authorization
   */
  removeExpiredAccountAuthorization(who: string): CallBuilder {
    return new CallBuilder((options) => {
      const tx =
        this.api.tx.TransactionStorage.remove_expired_account_authorization({
          who,
        })
      return this.submitTx(
        tx,
        "Failed to remove expired account authorization",
        ErrorCode.TRANSACTION_FAILED,
        options,
      )
    })
  }

  /**
   * Remove an expired preimage authorization
   *
   * Can be called by anyone (no special origin required).
   *
   * @param contentHash - Blake2b-256 hash of the expired authorization
   */
  removeExpiredPreimageAuthorization(contentHash: Uint8Array): CallBuilder {
    return new CallBuilder((options) => {
      const tx =
        this.api.tx.TransactionStorage.remove_expired_preimage_authorization({
          content_hash: Binary.toHex(contentHash),
        })
      return this.submitTx(
        tx,
        "Failed to remove expired preimage authorization",
        ErrorCode.TRANSACTION_FAILED,
        options,
      )
    })
  }

  /**
   * Estimate authorization needed for storing data
   */
  estimateAuthorization(dataSize: number): {
    transactions: number
    bytes: number
  } {
    return this.preparer.estimateAuthorization(dataSize)
  }
}
