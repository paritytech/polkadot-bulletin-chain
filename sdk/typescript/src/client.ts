// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * Async client with full transaction submission support
 */

import type { JsonRpcProvider } from "@polkadot-api/json-rpc-provider"
import { ss58Address } from "@polkadot-labs/hdkd-helpers"
import type { CID } from "multiformats/cid"
import { Binary, createClient, type PolkadotSigner } from "polkadot-api"
import type { BlobSource, SeekableSource } from "./blob-source.js"
import { type RenewShape, resolveRenewShape } from "./compat.js"
import {
  isStallError,
  type PipelineBootstrap,
  type PipelineItem,
  pipelineStore,
  readStoredAt,
} from "./pipeline.js"
import { BulletinPreparer } from "./preparer.js"
import {
  BulletinError,
  type ChunkerConfig,
  type ChunkPlan,
  CidCodec,
  type ClientConfig,
  DEFAULT_STORE_OPTIONS,
  ErrorCode,
  HashAlgorithm,
  type ProgressCallback,
  type ResolvedClientConfig,
  resolveClientConfig,
  type StoreOptions,
  type StoreResult,
  type StreamEstimate,
  TxStatus,
  type UploadCallback,
  type UploadEstimate,
  type UploadEstimateItem,
  type UploadEstimateOptions,
  type UploadItem,
  type UploadResult,
  UploadStatus,
  type WaitFor,
} from "./types.js"
import {
  calculateCid,
  cidToContentHashHex,
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
 * On-chain `TransactionRef` used by the renewal extrinsics — the PAPI
 * tagged-enum shape of the runtime's `TransactionRef` enum. `ContentHash`
 * requires a runtime that ships `TransactionRef`; its value is the 32-byte
 * content hash as a `0x`-prefixed hex string (PAPI encodes fixed-size binary
 * as `SizedHex` and rejects raw byte arrays).
 */
export type TransactionRef =
  | { type: "Position"; value: { block: number; index: number } }
  | { type: "ContentHash"; value: string }

/**
 * Caller-friendly reference to stored data for `renew()`/`forceRenew()`. The
 * variant is inferred from the shape: `{ block, index }` becomes `Position`;
 * a `Uint8Array` content hash becomes `ContentHash`.
 */
export type TransactionRefInput = { block: number; index: number } | Uint8Array

/** Convert a {@link TransactionRefInput} into the on-chain tagged enum. */
export function toTransactionRef(ref: TransactionRefInput): TransactionRef {
  if (ref instanceof Uint8Array)
    return { type: "ContentHash", value: Binary.toHex(ref) }
  return { type: "Position", value: { block: ref.block, index: ref.index } }
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
      // `renew` takes a `TransactionRef` on current runtimes and `(block, index)`
      // on older ones; the SDK detects which via the compat registry.
      renew(
        args: { block: number; index: number } | { entry: TransactionRef },
      ): PapiTransaction
      // Only present on runtimes that ship `TransactionRef`.
      force_renew?(args: { entry: TransactionRef }): PapiTransaction
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
    /** Utility pallet — used by `authorizeAccount(entries[])` for atomic
     * multi-account grants via `batch_all`. */
    Utility: {
      batch_all(args: { calls: unknown[] }): PapiTransaction
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
 * use the pipelined engine with its own providers factory.
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
 * Both `BulletinClient` and `MockBulletinClient` implement this interface.
 */
/** Single authorization-grant entry, used for the batched form of
 * {@link BulletinClientInterface.authorizeAccount}. */
export interface AuthorizeAccountEntry {
  who: string
  transactions: number
  bytes: bigint
}

export interface BulletinClientInterface {
  /** Lazy submission of a prepared estimate (from `estimateUpload(source)`):
   *  fetches chunk bytes on demand from the {@link SeekableSource}, freeing
   *  them on finalization. The single primitive for streamed/file uploads. */
  submit(estimate: StreamEstimate, source: SeekableSource): SubmitBuilder
  authorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
  ): AuthCallBuilder
  authorizeAccount(entries: AuthorizeAccountEntry[]): AuthCallBuilder
  authorizePreimage(contentHash: Uint8Array, maxSize: bigint): AuthCallBuilder
  renew(ref: TransactionRefInput): CallBuilder
  forceRenew(ref: TransactionRefInput): CallBuilder
  refreshAccountAuthorization(who: string): AuthCallBuilder
  refreshPreimageAuthorization(contentHash: Uint8Array): AuthCallBuilder
  removeExpiredAccountAuthorization(who: string): CallBuilder
  removeExpiredPreimageAuthorization(contentHash: Uint8Array): CallBuilder
  estimateAuthorization(dataSize: number): {
    transactions: number
    bytes: number
  }
  estimateUpload(
    input: UploadItem[] | BlobSource,
    options?: UploadEstimateOptions,
  ): Promise<StreamEstimate>
  /** Release resources held on behalf of this client (e.g. underlying PAPI client). */
  destroy(): Promise<void>
}

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
   * unit's content hash — its CID's multihash digest, using whatever hash
   * algorithm that unit was prepared with (Blake2b-256 by default) — to be
   * preimage-authorized on-chain beforehand (typically via
   * `authorizePreimage()`). Works for any estimate — a single item, a batch,
   * or a chunked file (chunks + manifest).
   *
   * Progress events (ItemStarted/InBlock/Finalized/Failed) fire per unit
   * with `index` matching its position in the source.
   *
   * When combined with `ensureAuthorized()`, the pre-flight checks each
   * unit's `Authorizations<Preimage(content_hash)>` entry instead of the
   * signer's account authorization. Duplicate content hashes are deduped
   * before the RPC queries.
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

/** Dispatch callback for the `submit(estimate, source)` execution path. */
type SubmitDispatch = (
  waitFor: WaitFor,
  onEvent: UploadCallback | undefined,
  checkAuth: boolean,
  unsigned: boolean,
) => Promise<UploadResult>

/**
 * Builder for `submit(estimate, source)`. Resolves with the CIDs in unit order
 * (`cids[i]` ↔ the i-th unit; the last CID is the manifest root when a manifest
 * is present). `.asUnsigned()` submits preimage-authorized bare extrinsics.
 */
export class SubmitBuilder extends BaseUploadBuilder<UploadResult> {
  constructor(private dispatch: SubmitDispatch) {
    super()
  }

  async send(): Promise<UploadResult> {
    return this.dispatch(
      this.waitFor,
      this.callback,
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
 *   .renew({ block: blockNumber, index })
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

/**
 * Reject uploads whose items have duplicate content hashes. The pipeline's
 * `TransactionByContentHash`-based reconciler identifies items by their
 * content hash; two items with the same hash would map to one TBCH entry,
 * making per-item finalization undecidable. Catch this at submission time
 * with a clear error rather than silently stalling.
 *
 * Returns the index of the first duplicate so the caller can act on it.
 */
/** Wrap an in-memory {@link UploadItem} as a {@link PipelineItem} (resident
 *  bytes). The lazy path builds PipelineItems whose `getData` range-reads. */

function assertUniqueContentHashes(cids: CID[], skip?: Set<number>): void {
  const seen = new Map<string, number>()
  for (let i = 0; i < cids.length; i++) {
    if (skip?.has(i)) continue
    const hex = normalizeHex(Binary.toHex(cids[i]!.multihash.digest))
    const prior = seen.get(hex)
    if (prior !== undefined) {
      throw new BulletinError(
        `submit(): item ${i} has the same content hash as item ${prior} — the SDK identifies items by content hash and can't distinguish duplicates. Pass the default dedupInput estimate (which skips duplicates), or store the same data in separate submit() calls.`,
        ErrorCode.INVALID_CONFIG,
      )
    }
    seen.set(hex, i)
  }
}

/**
 * Completed-with-failures guard. The pipeline resolves even when some items
 * exhausted their retry budget (it completes the rest), so a resolved run is
 * not necessarily a full success — a failed index's cid is NOT on chain.
 * Surface that as `UPLOAD_INCOMPLETE` (with the caller's original indices in
 * `cause.failedIndices`) rather than returning a silent full-cids success.
 * `newToOriginal` maps pipeline (compacted) indices back to caller indices.
 */
function assertNoFailedItems(
  failed: number[],
  newToOriginal: number[],
  total: number,
): void {
  if (failed.length === 0) return
  const failedOriginal = failed
    .map((i) => newToOriginal[i])
    .filter((v): v is number => v !== undefined)
  throw new BulletinError(
    `upload incomplete: ${failedOriginal.length} of ${total} items failed permanently (indices ${failedOriginal.join(", ")}); see ItemFailed events for per-item causes`,
    ErrorCode.UPLOAD_INCOMPLETE,
    { failedIndices: failedOriginal },
  )
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
 * Bulletin Chain client. Owns its PAPI connection (built from
 * `providers()[0]`) and submits storage + authorization extrinsics.
 *
 * @example
 * ```typescript
 * import { getWsProvider } from 'polkadot-api/ws';
 * import { BulletinClient, blobFromBytes } from '@parity/bulletin-sdk';
 *
 * const client = new BulletinClient({
 *   providers: () => [getWsProvider('ws://localhost:9944')],
 *   uploadSigner: signer,
 *   descriptor: bulletinDescriptor, // optional; omit to use getUnsafeApi()
 * });
 *
 * const src = blobFromBytes(data);
 * const { cids } = await client.submit(await client.estimateUpload(src), src).send();
 * const rootCid = cids[cids.length - 1]; // manifest root (or the lone chunk's CID)
 * ```
 */
/**
 * Options for constructing a {@link BulletinClient}.
 *
 * The SDK builds and owns the internal `PolkadotClient` from
 * `providers()[0]`, derives the typed API via `getTypedApi(descriptor)`,
 * and exposes both as `client.api` / `client.submitAndWatch`.
 * `client.destroy()` tears it all down.
 */
export interface BulletinClientOptions extends Partial<ClientConfig> {
  /**
   * PAPI chain descriptor (generated by `papi add`). Optional — when
   * omitted the SDK falls back to `PolkadotClient.getUnsafeApi()` which
   * still works at runtime but loses compile-time chain types. Pass
   * your generated descriptor for full TypeScript safety.
   */
  // biome-ignore lint/suspicious/noExplicitAny: descriptor is generated per-chain — opaque to the SDK
  descriptor?: any
  /**
   * Upload signer. Optional — pass `undefined` for unsigned-only mode.
   * Signed paths throw `UNSUPPORTED_OPERATION` on a signer-less client.
   */
  uploadSigner?: PolkadotSigner
}

export class BulletinClient implements BulletinClientInterface {
  /** Typed PAPI API for direct queries (also used by the SDK internally). */
  public api: BulletinTypedApi
  /** Stream-watch submission, exposed for advanced callers. */
  public submitAndWatch: SubmitAndWatchFn
  /** Upload signer (undefined → unsigned-only mode). */
  public signer: PolkadotSigner | undefined
  /** Client configuration */
  public config: ResolvedClientConfig
  /** Offline operations (chunking, CID calculation, estimation) */
  private preparer: BulletinPreparer
  /** Internal PAPI client owned by the SDK — torn down on `destroy()`. */
  // biome-ignore lint/suspicious/noExplicitAny: PolkadotClient type omitted to avoid hard PAPI version coupling here
  private papiClient: any
  /**
   * Mutable pipeline bootstrap cache shared across every upload from this
   * client. Populated on the first successful `pipelineStore` call (metadata
   * fetch + offline-API build); subsequent calls skip the round-trip. Survives
   * the lifetime of the client.
   */
  private pipelineBootstrap: PipelineBootstrap = {}
  /** Cached `renew` shape resolution for the connected chain (see compat.ts). */
  private renewShapePromise?: Promise<RenewShape>

  /**
   * Construct a client.
   *
   * The SDK creates and owns an internal PAPI client from
   * `options.providers()[0]` and a typed API from `options.descriptor`.
   * Both stay accessible via `client.api` / `client.submitAndWatch` for
   * callers who need to run their own queries. `client.destroy()` tears
   * them down.
   */
  constructor(options: BulletinClientOptions) {
    if (!options.providers) {
      throw new BulletinError(
        "BulletinClient: `providers` factory is required",
        ErrorCode.INVALID_CONFIG,
      )
    }
    const initialProviders = options.providers()
    if (!initialProviders.length) {
      throw new BulletinError(
        "BulletinClient: `providers()` must return at least one JsonRpcProvider",
        ErrorCode.INVALID_CONFIG,
      )
    }
    this.papiClient = createClient(initialProviders[0])
    this.api = (options.descriptor
      ? this.papiClient.getTypedApi(options.descriptor)
      : this.papiClient.getUnsafeApi()) as unknown as BulletinTypedApi
    this.submitAndWatch = this.papiClient
      .submitAndWatch as unknown as SubmitAndWatchFn
    this.signer = options.uploadSigner
    this.config = resolveClientConfig({
      providers: options.providers,
      authorizerSigner: options.authorizerSigner,
      txTimeout: options.txTimeout,
      blockLimits: options.blockLimits,
      defaultChunkSize: options.defaultChunkSize,
      chunkingThreshold: options.chunkingThreshold,
      createManifest: options.createManifest,
      submissionStrategy: options.submissionStrategy,
    })
    this.preparer = new BulletinPreparer({
      defaultChunkSize: this.config.defaultChunkSize,
      createManifest: this.config.createManifest,
      chunkingThreshold: this.config.chunkingThreshold,
    })
  }

  /**
   * Release the SDK's internal PAPI client (closes its WS connection and
   * clears subscriptions). Idempotent after the first call.
   */
  async destroy(): Promise<void> {
    if (this.papiClient) {
      try {
        this.papiClient.destroy()
      } catch {
        /* ignore double-destroy */
      }
      this.papiClient = undefined
    }
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
   * every item has a non-expired `Authorizations<Preimage(content_hash)>`
   * entry on chain, where `content_hash` is the CID's multihash digest under
   * the item's chosen hash algorithm (Blake2b-256 by default). Preimage
   * authorization is what the runtime checks for
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
      ? cids.map(cidToContentHashHex)
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
   * Authorization-class methods (`authorizeAccount`, `authorizePreimage`,
   * `refreshAccountAuthorization`, `refreshPreimageAuthorization`) must
   * be signed by an explicit Authorizer key, not by the client's
   * primary upload signer. This guards against accidental use of the
   * upload account for permission grants.
   */
  private requireAuthorizerSigner(operation: string): PolkadotSigner {
    const s = this.config.authorizerSigner
    if (!s) {
      throw new BulletinError(
        `${operation} requires \`authorizerSigner\` to be set in ClientConfig`,
        ErrorCode.UNSUPPORTED_OPERATION,
      )
    }
    return s
  }

  /** Resolve the provider factory, throwing a clear error if unset. */
  private requireProviders(operation: string): () => JsonRpcProvider[] {
    const p = this.config.providers
    if (!p) {
      throw new BulletinError(
        `${operation} requires \`providers\` to be set in ClientConfig`,
        ErrorCode.UNSUPPORTED_OPERATION,
      )
    }
    return p
  }

  /**
   * Create a store transaction.
   *
   * The chain defaults to Raw (0x55) codec + Blake2b-256 hashing, so the plain
   * `store()` extrinsic is sufficient for the common case. We only use the heavier
   * `store_with_cid_config()` extrinsic when the user requests non-default settings.
   */

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
    signerOverride?: PolkadotSigner,
  ): Promise<{
    blockHash: string
    txHash: string
    blockNumber?: number
    txIndex?: number
    events?: RuntimeEvent[]
  }> {
    const useSigner = signerOverride ?? this.signer
    if (!useSigner) {
      throw new BulletinError(
        "signAndSubmitWithProgress requires a signer",
        ErrorCode.UNSUPPORTED_OPERATION,
      )
    }
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

      const subscription = tx.signSubmitAndWatch(useSigner).subscribe({
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
    signerOverride?: PolkadotSigner,
  ): Promise<TransactionReceipt> {
    try {
      const waitFor = options?.waitFor ?? "in_block"
      const result = await this.signAndSubmitWithProgress(
        tx,
        options?.onProgress,
        waitFor,
        undefined,
        signerOverride,
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
   * Lazy submission of a prepared {@link StreamEstimate} (from
   * `estimateUpload(source)`). Stores the estimate's chunks — skipping any it
   * marked `alreadyStored` — fetching each chunk's bytes on demand via
   * `source.read(offset, size)` and holding only the in-flight window in memory
   * (freed on finalization). The manifest, if any, is submitted last.
   *
   * Flow: `const est = await client.estimateUpload(source)` → (preview /
   * authorize from `est.transactions`/`est.bytes`) → `client.submit(est, source)`.
   */
  submit(estimate: StreamEstimate, source: SeekableSource): SubmitBuilder {
    return new SubmitBuilder((wf, oe, ca, un) =>
      this.submitEstimateImpl(estimate, source, wf, oe, ca, un),
    )
  }

  private async submitEstimateImpl(
    estimate: StreamEstimate,
    source: SeekableSource,
    waitFor: WaitFor,
    onEvent: UploadCallback | undefined,
    checkAuth: boolean,
    unsigned: boolean,
  ): Promise<UploadResult> {
    const plan = estimate.plan
    // The plan's CIDs were hashed from the estimate-pass bytes; submit reads
    // bytes from `source` and reconciles against those CIDs. A size mismatch
    // means the source changed or differs from the one estimated — the chain
    // would store one payload while the pipeline reconciles another hash (a
    // silent stall, or wrong content paid for). Fail fast on the cheap check.
    if (source.size !== plan.totalSize) {
      throw new BulletinError(
        `submit(): source size ${source.size} does not match the estimate (${plan.totalSize} bytes) — the source changed or differs from the one passed to estimateUpload(). Re-run estimateUpload(source) with the current source.`,
        ErrorCode.INVALID_CONFIG,
      )
    }
    // One PipelineItem per unit, honoring per-unit codec/hashAlgo (file chunks
    // default Raw/Blake2b; items-as-is carry their own). Bytes fetched lazily.
    const items: PipelineItem[] = plan.chunkCids.map((_, i) => ({
      size: plan.chunkSizes[i] as number,
      codec: plan.codecs?.[i] ?? CidCodec.Raw,
      hashAlgo: plan.hashAlgos?.[i] ?? HashAlgorithm.Blake2b256,
      getData: () =>
        source.read(plan.offsets[i] as number, plan.chunkSizes[i] as number),
    }))
    const cids: CID[] = [...plan.chunkCids]
    if (plan.rootCid && plan.manifestData) {
      const manifestData = plan.manifestData
      items.push({
        size: manifestData.length,
        codec: CidCodec.DagPb,
        getData: async () => manifestData,
      })
      cids.push(plan.rootCid)
    }
    // Honor the estimate's dedup: skip units it collapsed as within-input
    // duplicates (the first occurrence carries that content) and units already
    // on chain. Skipping — rather than rejecting — is what makes the estimate a
    // valid submission plan: the same content is still stored exactly once.
    const preSkipped = new Set<number>([
      ...estimate.alreadyStored,
      ...estimate.duplicateIndices,
    ])
    // Guard the SUBMITTED (non-skipped) set: two live items sharing a content
    // hash collide in the TBCH reconciler. Estimate-collapsed dups are skipped
    // above, so this only fires when dedup was disabled and genuine dups remain.
    assertUniqueContentHashes(cids, preSkipped)
    const hashes = cids.map(cidToContentHashHex)

    if (unsigned) {
      return this.runUnsignedSubmit(
        items,
        cids,
        preSkipped,
        checkAuth,
        waitFor,
        onEvent,
      )
    }
    this.requireSigner("submit()")
    if (checkAuth) await this.ensureAuthorizedOnChain()
    // runSignedRetry's per-attempt TBCH check covers anything that landed
    // between estimate and submit.
    return this.runSignedRetry(
      items,
      cids,
      hashes,
      preSkipped,
      waitFor,
      onEvent,
    )
  }

  /** Unsigned (preimage-authorized) submission of prepared items. */
  private async runUnsignedSubmit(
    items: PipelineItem[],
    allItemCids: CID[],
    preSkipped: Set<number>,
    checkAuth: boolean,
    waitFor: WaitFor,
    onEvent: UploadCallback | undefined,
  ): Promise<UploadResult> {
    const providers = this.requireProviders("submit().asUnsigned()")
    // Drop skipped units (estimate-collapsed duplicates, or already on chain)
    // and remap pipeline event indices back to the caller's original indices.
    // The unsigned path has no retry loop, so a one-shot filter suffices (cf.
    // runSignedRetry's newToOriginal).
    const submitItems: PipelineItem[] = []
    const submitCids: CID[] = []
    const newToOriginal: number[] = []
    for (let i = 0; i < items.length; i++) {
      if (preSkipped.has(i)) continue
      submitItems.push(items[i] as PipelineItem)
      submitCids.push(allItemCids[i] as CID)
      newToOriginal.push(i)
    }
    if (checkAuth) await this.ensurePreimagesAuthorized([], submitCids)
    const remapEvent: UploadCallback | undefined = onEvent
      ? (ev) =>
          onEvent({
            ...ev,
            index: newToOriginal[ev.index] ?? ev.index,
            total: items.length,
          })
      : undefined
    try {
      const result = await pipelineStore(this.api, undefined, submitItems, {
        providers,
        blockLimits: this.config.blockLimits,
        completeOn: waitFor === "in_block" ? "best" : "finalized",
        bootstrap: this.pipelineBootstrap,
        precomputedCids: submitCids,
        submissionStrategy: this.config.submissionStrategy,
        onEvent: remapEvent,
      })
      // Unsigned items carry no per-item retry budget today (`failed` stays
      // empty; a stuck item stalls instead) — same guard as the signed path
      // so the contract holds if that changes.
      assertNoFailedItems(result.failed, newToOriginal, items.length)
      // Return the full CID set (matches the signed path); skipped units share
      // content with a submitted unit or are already stored.
      return { cids: allItemCids }
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
   * Shared signed-submission retry loop over prepared items. Resumes across
   * transient stalls by original index (skipping landed items) and carries
   * nonces forward. Drives the `submit(estimate, source)` path — `items`
   * load their bytes lazily via `source.read`.
   */
  private async runSignedRetry(
    items: PipelineItem[],
    allItemCids: CID[],
    allContentHashesHex: string[],
    preSkipped: Set<number>,
    waitFor: WaitFor,
    onEvent: UploadCallback | undefined,
  ): Promise<UploadResult> {
    // Shared bootstrap cache (one per client instance, see field decl)
    // is reused across uploads AND across retry attempts within one upload.
    const bootstrap = this.pipelineBootstrap

    // Retry on transient stalls; resume by filtering out items that
    // already landed (tracked by original index, not by count — items can
    // land non-contiguously under hijack/race conditions). CIDs are
    // deterministic from input so we return `allItemCids` directly.
    //
    // Exactly-once broadcast guarantee within a single uploadItems
    // session: before every retry attempt, query TBCH for each
    // not-yet-finalized item. If on-chain (regardless of who put it
    // there during *this session*), treat as finalized and skip
    // re-broadcast. Without this, an item that landed in a best block
    // before STORE_STALLED fired would not appear in
    // `error.cause.finalizedIndices` (only finalized items do), and the
    // retry would resubmit + double-pay.
    const maxRetries = 3
    let attempt = 0
    // Items pre-skipped via `skipExisting` are pre-populated here so the
    // retry loop's slicing and the early-return short-circuit treat them
    // exactly like items that finalized during an earlier attempt.
    const finalizedOriginal = new Set<number>(preSkipped)
    // Persisted nonces across retry boundaries, indexed by ORIGINAL item
    // index. Populated from `error.cause.itemNonce` on each stall, mapped
    // back from the prior call's compacted indices via `newToOriginal`.
    // Seeding the next pipelineStore call with these prevents nonce
    // re-assignment for items whose previous submission is still alive in
    // the pool — without this, a retry would double-claim higher nonces
    // and create duplicate submissions for the same content, charging the
    // user twice. Items the within-call hijack detector cleared have
    // `undefined` here; the new call's first wave will assign them fresh.
    const originalItemNonces: Array<number | undefined> = new Array(
      items.length,
    ).fill(undefined)
    while (true) {
      // Pre-retry TBCH dedup: items already on chain are skipped. Always
      // query at PAPI's `"finalized"` sentinel — resolves through
      // chainHead_v1_storage against the currently-tracked finalized
      // block, so the answer is reorg-stable and never UnknownBlock-
      // fails on non-archive nodes.
      if (attempt > 0) {
        const pendingIndexes: number[] = []
        const pendingHashes: string[] = []
        for (let i = 0; i < items.length; i++) {
          if (finalizedOriginal.has(i)) continue
          pendingIndexes.push(i)
          pendingHashes.push(allContentHashesHex[i] as string)
        }
        if (pendingIndexes.length > 0) {
          const entries = await Promise.all(
            pendingHashes.map((h) => readStoredAt(this.api, h, "finalized")),
          )
          // Synthesized events carry the finalized tip's hash — best-effort
          // observation context; the authoritative renewal slot is
          // `(blockNumber, transactionIndex)` from TBCH.
          let tipHash = ""
          if (onEvent && entries.some(Boolean)) {
            try {
              tipHash = (await this.papiClient.getFinalizedBlock()).hash ?? ""
            } catch {
              /* keep "" — context only, never worth failing the retry */
            }
          }
          for (let k = 0; k < pendingIndexes.length; k++) {
            const entry = entries[k]
            if (!entry) continue
            const i = pendingIndexes[k] as number
            finalizedOriginal.add(i)
            onEvent?.({
              type: UploadStatus.ItemFinalized,
              index: i,
              total: items.length,
              cid: allItemCids[i] as CID,
              blockHash: tipHash,
              blockNumber: entry.blockNumber,
              transactionIndex: entry.transactionIndex,
            })
          }
        }
      }
      const remaining: PipelineItem[] = []
      const remainingCids: CID[] = []
      const newToOriginal: number[] = []
      for (let i = 0; i < items.length; i++) {
        if (finalizedOriginal.has(i)) continue
        newToOriginal.push(i)
        remaining.push(items[i] as PipelineItem)
        remainingCids.push(allItemCids[i] as CID)
      }
      if (remaining.length === 0) break
      // Build a seed nonce array aligned with `remaining` (compacted
      // indices), sourced from `originalItemNonces` (which lives in
      // original-index space). Items whose seed was cleared by the
      // within-call hijack detector — or that have no carry-over yet —
      // get `undefined`, which tells pipelineStore to assign fresh
      // from the wave's poolNonce floor.
      const seedItemNonces: (number | undefined)[] = newToOriginal.map(
        (origIdx) => originalItemNonces[origIdx],
      )
      try {
        const result = await pipelineStore(this.api, this.signer!, remaining, {
          providers: this.requireProviders("upload()"),
          blockLimits: this.config.blockLimits,
          completeOn: waitFor === "in_block" ? "best" : "finalized",
          bootstrap,
          precomputedCids: remainingCids,
          submissionStrategy: this.config.submissionStrategy,
          seedItemNonces,
          onEvent: onEvent
            ? (ev) =>
                onEvent({
                  ...ev,
                  index: newToOriginal[ev.index] as number,
                  // Pipeline events carry the compacted attempt-local total;
                  // callers see the caller-space total (cf. runUnsignedSubmit).
                  total: items.length,
                })
            : undefined,
        })
        assertNoFailedItems(result.failed, newToOriginal, items.length)
        break
      } catch (error) {
        if (isStallError(error) && attempt < maxRetries) {
          for (const newIdx of error.cause.finalizedIndices) {
            const originalIdx = newToOriginal[newIdx]
            if (originalIdx !== undefined) finalizedOriginal.add(originalIdx)
          }
          // Carry forward each in-flight item's nonce so the next call
          // re-broadcasts at the same slot (same nonce, fresh era
          // anchor). Items whose nonce was cleared by hijack detection
          // in the stalled call arrive here as `undefined` and stay
          // `undefined`.
          const stalledNonces = error.cause.itemNonce ?? []
          for (let newIdx = 0; newIdx < stalledNonces.length; newIdx++) {
            const origIdx = newToOriginal[newIdx]
            if (origIdx === undefined) continue
            const n = stalledNonces[newIdx]
            originalItemNonces[origIdx] = n
          }
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
   * Authorize an account to store data
   *
   * @param who - Account address to authorize
   * @param transactions - Number of transactions to authorize
   * @param bytes - Maximum bytes to authorize
   */
  /**
   * Authorize one or many accounts to store data. With a single
   * `(who, transactions, bytes)` triple it dispatches a single
   * `TransactionStorage.authorize_account`. With an array of entries it
   * wraps them in `Utility.batch_all` — atomic: either every
   * authorization is applied or none of them are.
   *
   * Signed by `config.authorizerSigner` if set, otherwise by the
   * client's primary upload signer.
   */
  authorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
  ): AuthCallBuilder
  authorizeAccount(entries: AuthorizeAccountEntry[]): AuthCallBuilder
  authorizeAccount(
    whoOrEntries: string | AuthorizeAccountEntry[],
    transactions?: number,
    bytes?: bigint,
  ): AuthCallBuilder {
    return new AuthCallBuilder((options) => {
      const signer = this.requireAuthorizerSigner("authorizeAccount()")
      const authTx = this.buildAuthorizeAccountTx(
        whoOrEntries,
        transactions,
        bytes,
      )
      return this.submitTx(
        this.maybeSudo(authTx, options?.sudo),
        "Failed to authorize account",
        ErrorCode.AUTHORIZATION_FAILED,
        options,
        signer,
      )
    })
  }

  private buildAuthorizeAccountTx(
    whoOrEntries: string | AuthorizeAccountEntry[],
    transactions: number | undefined,
    bytes: bigint | undefined,
  ): PapiTransaction {
    if (typeof whoOrEntries === "string") {
      if (transactions === undefined || bytes === undefined) {
        throw new BulletinError(
          "authorizeAccount(who, transactions, bytes) requires all 3 args",
          ErrorCode.INVALID_CONFIG,
        )
      }
      return this.api.tx.TransactionStorage.authorize_account({
        who: whoOrEntries,
        transactions,
        bytes,
      })
    }
    if (whoOrEntries.length === 0) {
      throw new BulletinError(
        "authorizeAccount(entries) requires at least one entry",
        ErrorCode.INVALID_CONFIG,
      )
    }
    if (whoOrEntries.length === 1) {
      const e = whoOrEntries[0] as AuthorizeAccountEntry
      return this.api.tx.TransactionStorage.authorize_account({
        who: e.who,
        transactions: e.transactions,
        bytes: e.bytes,
      })
    }
    const calls = whoOrEntries.map(
      (e) =>
        this.api.tx.TransactionStorage.authorize_account({
          who: e.who,
          transactions: e.transactions,
          bytes: e.bytes,
        }).decodedCall,
    )
    return this.api.tx.Utility.batch_all({ calls })
  }

  /**
   * Authorize a preimage (by content hash) to be stored
   *
   * @param contentHash - Blake2b-256 hash of the content to authorize
   * @param maxSize - Maximum size in bytes for the content
   */
  authorizePreimage(contentHash: Uint8Array, maxSize: bigint): AuthCallBuilder {
    return new AuthCallBuilder((options) => {
      const signer = this.requireAuthorizerSigner("authorizePreimage()")
      const authTx = this.api.tx.TransactionStorage.authorize_preimage({
        content_hash: Binary.toHex(contentHash),
        max_size: maxSize,
      })
      return this.submitTx(
        this.maybeSudo(authTx, options?.sudo),
        "Failed to authorize preimage",
        ErrorCode.AUTHORIZATION_FAILED,
        options,
        signer,
      )
    })
  }

  /**
   * Schedule a one-shot renewal of stored data.
   *
   * The renewal fires once when the data reaches its retention boundary; it
   * does not renew synchronously. For immediate renewal use {@link forceRenew}.
   *
   * @param ref - A `{ block, index }` position (from the `Stored`/`Renewed`
   *   event) or a `Uint8Array` content hash. Content-hash renewal requires a
   *   `TransactionRef` runtime.
   */
  renew(ref: TransactionRefInput): CallBuilder {
    return new CallBuilder(async (options) => {
      // Registry dispatch (see compat.ts): the connected chain's checksum
      // for `TransactionStorage.renew` selects the encoder — identification
      // first, never trial-encoding; unknown shapes fail closed. Both arms
      // encode via the unsafe api (codecs built from the live metadata) so
      // a stale caller descriptor can't veto a compatible chain.
      const entry = toTransactionRef(ref)
      const shape = await this.renewShape()
      const renewTx = this.papiClient.getUnsafeApi().tx.TransactionStorage
        .renew as (args: object) => PapiTransaction
      let tx: PapiTransaction
      if (shape === "transaction-ref") {
        tx = renewTx({ entry })
      } else if (entry.type === "Position") {
        // Pre-`TransactionRef` runtimes take the position fields directly.
        tx = renewTx(entry.value)
      } else {
        throw new BulletinError(
          "content-hash renewal is not supported by this runtime",
          ErrorCode.UNSUPPORTED_OPERATION,
        )
      }
      return this.submitTx(
        tx,
        "Failed to renew",
        ErrorCode.TRANSACTION_FAILED,
        options,
      )
    })
  }

  /**
   * Immediately renew stored data, extending its retention from the current
   * block.
   *
   * Requires a runtime that ships `TransactionRef` / `force_renew`; older
   * runtimes reject with `UNSUPPORTED_OPERATION`.
   *
   * @param ref - A `{ block, index }` position or a `Uint8Array` content hash.
   */
  forceRenew(ref: TransactionRefInput): CallBuilder {
    return new CallBuilder(async (options) => {
      const ts = this.papiClient.getUnsafeApi().tx.TransactionStorage
      if ((await this.renewShape()) !== "transaction-ref" || !ts.force_renew) {
        throw new BulletinError(
          "force_renew is not supported by this runtime",
          ErrorCode.UNSUPPORTED_OPERATION,
        )
      }
      const tx = (ts.force_renew as (args: object) => PapiTransaction)({
        entry: toTransactionRef(ref),
      })
      return this.submitTx(
        tx,
        "Failed to force renew",
        ErrorCode.TRANSACTION_FAILED,
        options,
      )
    })
  }

  /**
   * Which `renew` shape the connected chain speaks, resolved once per client
   * by checksumming the live metadata (one RPC + a local hash — see
   * compat.ts). A failed resolution is not cached, so a transient RPC error
   * retries on the next call.
   */
  private renewShape(): Promise<RenewShape> {
    if (!this.renewShapePromise) {
      const resolved = (async () => {
        const metadataApis = this.papiClient.getUnsafeApi().apis.Metadata
        // OpaqueMetadata; v15 preferred, default version as fallback.
        const opaque =
          (await metadataApis.metadata_at_version(15)) ??
          (await metadataApis.metadata())
        // The unsafe api decodes OpaqueMetadata as plain bytes; accept a
        // Binary-like too in case that representation changes.
        const bytes: Uint8Array =
          opaque instanceof Uint8Array ? opaque : opaque.asBytes()
        return resolveRenewShape(bytes)
      })()
      resolved.catch(() => {
        if (this.renewShapePromise === resolved) {
          this.renewShapePromise = undefined
        }
      })
      this.renewShapePromise = resolved
    }
    return this.renewShapePromise
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
      const signer = this.requireAuthorizerSigner(
        "refreshAccountAuthorization()",
      )
      const authTx =
        this.api.tx.TransactionStorage.refresh_account_authorization({ who })
      return this.submitTx(
        this.maybeSudo(authTx, options?.sudo),
        "Failed to refresh account authorization",
        ErrorCode.AUTHORIZATION_FAILED,
        options,
        signer,
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
      const signer = this.requireAuthorizerSigner(
        "refreshPreimageAuthorization()",
      )
      const authTx =
        this.api.tx.TransactionStorage.refresh_preimage_authorization({
          content_hash: Binary.toHex(contentHash),
        })
      return this.submitTx(
        this.maybeSudo(authTx, options?.sudo),
        "Failed to refresh preimage authorization",
        ErrorCode.AUTHORIZATION_FAILED,
        options,
        signer,
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

  /**
   * Compute the dispatch plan and resource cost for a batch of upload
   * items WITHOUT actually submitting anything. Returns:
   *
   * - per-item CID + skip-reason
   * - aggregate `transactions` / `bytes` the chain will charge to the
   *   account's authorization (after the requested deduplication)
   *
   * By default duplicates within the input are collapsed (the chain
   * dedupes by content_hash anyway, so charging twice is wasteful). Pass
   * `skipExisting: true` to also query the chain's `TransactionByContentHash`
   * and exclude items already on chain (one RPC per unique content). The
   * returned estimate carries the skip set forward to `submit()`.
   *
   * Use this to size `authorizeAccount` before paying, or to preview the
   * cost of an upload in a UI.
   */
  async estimateUpload(
    input: UploadItem[] | BlobSource,
    options: UploadEstimateOptions = {},
  ): Promise<StreamEstimate> {
    return Array.isArray(input)
      ? this.estimateUploadItems(input, options)
      : this.estimateUploadStream(input, options)
  }

  private async estimateUploadItems(
    items: UploadItem[],
    options: UploadEstimateOptions,
  ): Promise<StreamEstimate> {
    for (let i = 0; i < items.length; i++) {
      if (items[i]!.data.length === 0) {
        throw new BulletinError(
          `Item ${i} has empty data`,
          ErrorCode.EMPTY_DATA,
        )
      }
    }
    const itemCids = await Promise.all(
      items.map((item) =>
        calculateCid(
          item.data,
          item.codec ?? CidCodec.Raw,
          item.hashAlgo ?? HashAlgorithm.Blake2b256,
        ),
      ),
    )
    const sizes = items.map((item) => item.data.length)
    // Items-as-is plan: one unit per item, per-item codec/hashAlgo, no
    // manifest. Offsets index into `blobFromItems(items)` at submit time.
    const offsets: number[] = []
    let total = 0
    for (const s of sizes) {
      offsets.push(total)
      total += s
    }
    const plan: ChunkPlan = {
      chunkCids: itemCids,
      chunkSizes: sizes,
      offsets,
      codecs: items.map((it) => it.codec ?? CidCodec.Raw),
      hashAlgos: items.map((it) => it.hashAlgo ?? HashAlgorithm.Blake2b256),
      totalSize: total,
      chunkSize: 0,
    }
    const estimate = await this.assembleEstimate(itemCids, sizes, options)
    return { ...estimate, plan }
  }

  /**
   * Streamed estimate: plan the source in O(chunkSize) memory (chunk CIDs +
   * sizes + manifest), then run the same dedup/skip logic over chunks +
   * manifest. The returned {@link ChunkPlan} can be reused to skip re-hashing.
   */
  private async estimateUploadStream(
    source: BlobSource,
    options: UploadEstimateOptions,
  ): Promise<StreamEstimate> {
    const plan = await this.preparer.planStream(source)
    const cids: CID[] = [...plan.chunkCids]
    const sizes: number[] = [...plan.chunkSizes]
    if (plan.rootCid && plan.manifestData) {
      cids.push(plan.rootCid)
      sizes.push(plan.manifestData.length)
    }
    const estimate = await this.assembleEstimate(cids, sizes, options)
    return { ...estimate, plan }
  }

  /** Shared dedup + chain-skip + assembly over precomputed CIDs and sizes. */
  private async assembleEstimate(
    itemCids: CID[],
    sizes: number[],
    options: UploadEstimateOptions,
  ): Promise<UploadEstimate> {
    const dedupInput = options.dedupInput ?? true
    const skipExisting = options.skipExisting ?? false
    const hashesHex = itemCids.map(cidToContentHashHex)

    // First-seen wins; later occurrences land in `duplicateIndices`.
    const duplicateIndices: number[] = []
    const firstSeen = new Map<string, number>()
    if (dedupInput) {
      for (let i = 0; i < itemCids.length; i++) {
        const h = hashesHex[i] as string
        if (firstSeen.has(h)) {
          duplicateIndices.push(i)
        } else {
          firstSeen.set(h, i)
        }
      }
    }

    // Optional chain dedup: TBCH lookup for each first-seen content_hash.
    const alreadyStored: number[] = []
    if (skipExisting) {
      // De-dup hashes before querying to avoid redundant RPCs for input dups.
      const uniqueHashIndexes = dedupInput
        ? Array.from(firstSeen.values())
        : itemCids.map((_, i) => i)
      const uniqueHashes = uniqueHashIndexes.map((i) => hashesHex[i] as string)
      const entries = await Promise.all(
        uniqueHashes.map((h) => readStoredAt(this.api, h)),
      )
      const onChainHashes = new Set<string>()
      for (let k = 0; k < uniqueHashes.length; k++) {
        if (entries[k]) onChainHashes.add(uniqueHashes[k] as string)
      }
      // Mark every index whose content is on chain — including duplicates: if
      // a duplicate's content_hash is on chain, it also wouldn't be submitted.
      for (let i = 0; i < itemCids.length; i++) {
        if (onChainHashes.has(hashesHex[i] as string)) alreadyStored.push(i)
      }
    }

    const dupSet = new Set(duplicateIndices)
    const onChainSet = new Set(alreadyStored)
    const skippedSet = new Set<number>([...duplicateIndices, ...alreadyStored])
    const toUpload: number[] = []
    const itemsOut: UploadEstimateItem[] = new Array(itemCids.length)
    let bytes = 0n
    for (let i = 0; i < itemCids.length; i++) {
      const dupOf = dedupInput && dupSet.has(i)
      const onChain = onChainSet.has(i)
      let skipReason: UploadEstimateItem["skipReason"]
      if (dupOf) skipReason = "duplicate_input"
      else if (onChain) skipReason = "already_on_chain"
      itemsOut[i] = {
        index: i,
        cid: itemCids[i] as CID,
        bytes: sizes[i] as number,
        ...(skipReason ? { skipReason } : {}),
      }
      if (!skippedSet.has(i)) {
        toUpload.push(i)
        bytes += BigInt(sizes[i] as number)
      }
    }

    return {
      total: itemCids.length,
      items: itemsOut,
      transactions: toUpload.length,
      bytes,
      duplicateIndices,
      alreadyStored,
      toUpload,
    }
  }
}
