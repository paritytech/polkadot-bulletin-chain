// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * Async client with full transaction submission support
 */

import type { CID } from "multiformats/cid"
import { Binary, type PolkadotSigner } from "polkadot-api"
import { BulletinPreparer } from "./preparer.js"
import {
  BulletinError,
  type ChunkedStoreResult,
  type ChunkerConfig,
  ChunkStatus,
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
  type WaitFor,
} from "./types.js"
import {
  estimateAuthorization,
  hashAlgorithmCodecToEnum,
  isNonDefaultCidConfig,
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
 * On-chain `TransactionRef` used by the renewal extrinsics.
 *
 * PAPI tagged-enum shape of the runtime's `TransactionRef` enum. `ContentHash`
 * requires a runtime that ships `TransactionRef`; its value is the 32-byte
 * content hash as a `0x`-prefixed hex string (PAPI represents fixed-size
 * binary values as `SizedHex`, and its encoder rejects raw byte arrays).
 */
export type TransactionRef =
  | { type: "Position"; value: { block: number; index: number } }
  | { type: "ContentHash"; value: string }

/**
 * Caller-friendly reference to stored data for `renew()`/`forceRenew()`.
 *
 * The variant is inferred from the shape: `{ block, index }` becomes
 * `Position`; a `Uint8Array` content hash becomes `ContentHash`.
 */
export type TransactionRefInput = { block: number; index: number } | Uint8Array

/** Convert a {@link TransactionRefInput} into the on-chain tagged enum. */
export function toTransactionRef(ref: TransactionRefInput): TransactionRef {
  if (ref instanceof Uint8Array)
    return { type: "ContentHash", value: Binary.toHex(ref) }
  return { type: "Position", value: { block: ref.block, index: ref.index } }
}

/** Which call shape the runtime's renewal extrinsics take. */
type RenewShape = "transactionRef" | "legacy"

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
      // on older ones; the SDK detects which at runtime.
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
  }
  /** Optional query interface for on-chain storage reads (e.g., authorization checks) */
  query?: {
    TransactionStorage: {
      Authorizations: {
        getValue(scope: { type: string; value: unknown }): Promise<
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
    }
  }
}

/**
 * Function type for submitting raw transactions to the chain.
 *
 * Matches the signature of `PolkadotClient.submit` from polkadot-api.
 * Pass `papiClient.submit` directly when constructing the client.
 */
export type SubmitFn = (
  transaction: Uint8Array,
  at?: string,
) => Promise<{
  ok: boolean
  block: { hash: string; number: number; index: number }
  txHash: string
  events: Array<{ type: string; value?: { type?: string; value?: unknown } }>
  dispatchError?: { type: string; value: unknown }
}>

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

/** Result of a completed sign-submit-watch cycle. */
interface SignSubmitResult {
  blockHash: string
  txHash: string
  blockNumber?: number
  txIndex?: number
  events?: RuntimeEvent[]
}

/**
 * PAPI's `InvalidTxError` for a transaction whose mortality era expired:
 * `{ type: "Invalid", value: { type: "AncientBirthBlock" } }`. Matched by
 * error name and shape (not instanceof) so it works across polkadot-api
 * module instances. Deliberately narrow: other invalid types (BadProof,
 * Stale, ...) must not be retried.
 */
function isAncientBirthBlockError(err: unknown): boolean {
  if (!(err instanceof Error) || err.name !== "InvalidTxError") return false
  const e = (err as { error?: { type?: unknown; value?: { type?: unknown } } })
    .error
  return e?.type === "Invalid" && e?.value?.type === "AncientBirthBlock"
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
  /** Store data with options (used internally by StoreBuilder) */
  storeWithOptions(
    data: Uint8Array,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
    chunkerConfig?: Partial<ChunkerConfig>,
  ): Promise<StoreResult>
  /** Store preimage-authorized content as unsigned transaction */
  storeWithPreimageAuth?(
    data: Uint8Array,
    options?: StoreOptions,
  ): Promise<StoreResult>
  store(data: Uint8Array): StoreBuilder
  authorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
  ): AuthCallBuilder
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
  /** Release resources held on behalf of this client (e.g. underlying PAPI client). */
  destroy(): Promise<void>
}

/**
 * Builder for store operations with fluent API
 *
 * @example
 * ```typescript
 * const result = await client
 *   .store(new TextEncoder().encode('Hello'))
 *   .withCodec(CidCodec.DagPb)
 *   .withHashAlgorithm('blake2b-256')
 *   .withCallback((event) => console.log('Progress:', event))
 *   .send();
 * ```
 */
export class StoreBuilder {
  private data: Uint8Array
  private options: StoreOptions = { ...DEFAULT_STORE_OPTIONS }
  private callback?: ProgressCallback
  private chunkerConfig?: Partial<ChunkerConfig>

  constructor(
    private executor: BulletinClientInterface,
    data: Uint8Array,
  ) {
    this.data = data
  }

  /** Set the CID codec. Accepts a `CidCodec` or a custom numeric multicodec code. */
  withCodec(codec: CidCodec | number): this {
    this.options.cidCodec = codec
    return this
  }

  /** Set the hash algorithm */
  withHashAlgorithm(algorithm: HashAlgorithm): this {
    this.options.hashingAlgorithm = algorithm
    return this
  }

  /** Set what to wait for before returning */
  withWaitFor(waitFor: WaitFor): this {
    this.options.waitFor = waitFor
    return this
  }

  /** Set progress callback for chunked uploads */
  withCallback(callback: ProgressCallback): this {
    this.callback = callback
    return this
  }

  /** Set chunk size (forces chunked upload path) */
  withChunkSize(chunkSize: number): this {
    this.chunkerConfig = { ...this.chunkerConfig, chunkSize }
    return this
  }

  /** Enable or disable DAG-PB manifest creation for chunked uploads (default: true) */
  withManifest(enabled: boolean): this {
    this.chunkerConfig = { ...this.chunkerConfig, createManifest: enabled }
    return this
  }

  /** Execute the store operation (signed transaction, uses account authorization) */
  async send(): Promise<StoreResult> {
    return this.executor.storeWithOptions(
      this.data,
      this.options,
      this.callback,
      this.chunkerConfig,
    )
  }

  /**
   * Execute store operation as unsigned transaction (for preimage-authorized content)
   *
   * Use this when the content has been pre-authorized via `authorizePreimage()`.
   * Unsigned transactions don't require fees and can be submitted by anyone.
   *
   * @example
   * ```typescript
   * // First authorize the content hash
   * const hash = blake2b256(data);
   * await client.authorizePreimage(hash, BigInt(data.length));
   *
   * // Anyone can now store this content without fees
   * const result = await client.store(data).sendUnsigned();
   * ```
   */
  async sendUnsigned(): Promise<StoreResult> {
    if (!this.executor.storeWithPreimageAuth) {
      throw new BulletinError(
        "Unsigned transactions not supported by this client",
        ErrorCode.UNSUPPORTED_OPERATION,
      )
    }
    return this.executor.storeWithPreimageAuth(this.data, this.options)
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
 *   .renew({ block, index })
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

/** Extract the transaction index from a Stored event in a list of runtime events */
function extractStoredIndex(events?: RuntimeEvent[]): number | undefined {
  if (!events) return undefined
  const storedEvent = events.find(
    (e) => e.type === "TransactionStorage" && e.value?.type === "Stored",
  )
  return storedEvent?.value?.value?.index
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
 * const bulletinClient = new AsyncBulletinClient(api, signer, papiClient.submit);
 *
 * // Store data
 * const result = await bulletinClient.store(data).send();
 * ```
 */
export class AsyncBulletinClient implements BulletinClientInterface {
  /** PAPI client for blockchain interaction */
  public api: BulletinTypedApi
  /** Signer for transaction signing */
  public signer: PolkadotSigner
  /** Submit function for broadcasting raw transactions (from PolkadotClient.submit) */
  public submit: SubmitFn
  /** Client configuration */
  public config: Required<ClientConfig>
  /** Offline operations (chunking, CID calculation, estimation) */
  private preparer: BulletinPreparer
  /** Optional teardown callback invoked by `destroy()` */
  private onDestroy?: () => void | Promise<void>

  /**
   * Create a new async client with PAPI client and signer
   *
   * The PAPI client must be configured with the correct chain metadata
   * for your Bulletin Chain node.
   *
   * @param api - Configured PAPI TypedApi instance
   * @param signer - Polkadot signer for transaction signing
   * @param submit - Raw transaction submit function (pass `papiClient.submit`)
   * @param config - Optional client configuration
   * @param onDestroy - Optional teardown callback. When provided, `destroy()`
   *   awaits it so callers (e.g. wrappers that own the underlying
   *   `PolkadotClient`) can route cleanup through this client.
   */
  constructor(
    api: BulletinTypedApi,
    signer: PolkadotSigner,
    submit: SubmitFn,
    config?: Partial<ClientConfig>,
    onDestroy?: () => void | Promise<void>,
  ) {
    this.api = api
    this.signer = signer
    this.submit = submit
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
   * Best-effort authorization check before a store submission.
   *
   * Allowances gate transaction *priority*, not acceptance — the chain never
   * rejects a store for an exhausted boost budget. So this only warns when the
   * budget looks insufficient and always proceeds. If `api.query` is not
   * available, the query fails, or returns nothing, it silently proceeds and
   * lets the chain validate.
   */
  private async checkAccountAuthorization(
    requiredTransactions: number,
    requiredBytes: number,
  ): Promise<void> {
    if (!this.api.query) return

    let auth:
      | {
          extent: {
            transactions: number
            transactions_allowance?: number
            bytes: bigint
            bytes_allowance?: bigint
          }
        }
      | undefined
    try {
      const { ss58Address } = await import("@polkadot-labs/hdkd-helpers")
      const address = ss58Address(this.signer.publicKey)

      auth = await this.api.query.TransactionStorage.Authorizations.getValue({
        type: "Account",
        value: address,
      })
    } catch {
      // Query failed (network error, etc.) — proceed and let the chain validate
      return
    }

    // Authorization not found — could be a timing issue (just authorized),
    // so proceed and let the chain validate rather than blocking
    if (!auth) return

    // Newer chains expose `*_allowance` (caps) alongside `transactions`/`bytes`
    // (consumed counters); older chains expose only the cap fields. Available
    // = allowance - consumed; falling back to the raw field when allowance
    // is absent keeps the SDK compatible with both shapes.
    const txAllowance = auth.extent.transactions_allowance
    const availableTransactions =
      txAllowance != null
        ? Math.max(0, txAllowance - auth.extent.transactions)
        : auth.extent.transactions
    const bytesAllowance = auth.extent.bytes_allowance
    const availableBytes =
      bytesAllowance != null
        ? Number(
            bytesAllowance > auth.extent.bytes
              ? bytesAllowance - auth.extent.bytes
              : 0n,
          )
        : Number(auth.extent.bytes)

    if (
      availableTransactions < requiredTransactions ||
      availableBytes < requiredBytes
    ) {
      console.warn(
        `Boost budget exhausted (need ${requiredTransactions} transactions / ${requiredBytes} bytes, ` +
          `have ${availableTransactions} / ${availableBytes}) - the store will proceed at lower priority`,
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
   * With "in_block" the promise resolves at first inclusion, but the
   * transaction stays broadcast and watched in the background until it
   * finalizes, so a reorg cannot silently drop it.
   *
   * Retries once if the submission dies of mortality-era expiry
   * (AncientBirthBlock): the node's fork-aware pool can silently lose a
   * ready transaction around a reorg (transaction_v1_broadcast gives no
   * drop feedback), and PAPI only reports the loss once the 64-block era
   * boundary finalizes. The retry is safe because past that finalized
   * boundary the original transaction's era check fails in every possible
   * block, so it can never be included and the re-signed submission (PAPI
   * anchors a fresh mortality era and nonce per subscribe) cannot
   * double-store. One retry only: a second consecutive era expiry means
   * ~13 minutes without inclusion, which is genuinely fatal.
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
  ): Promise<SignSubmitResult> {
    try {
      return await this.signAndSubmitAttempt(
        tx,
        progressCallback,
        waitFor,
        chunkIndex,
        true,
      )
    } catch (err) {
      if (!isAncientBirthBlockError(err)) throw err
      return this.signAndSubmitAttempt(
        tx,
        progressCallback,
        waitFor,
        chunkIndex,
        false,
      )
    }
  }

  /**
   * One sign-submit-watch cycle. `willRetryEraExpiry` suppresses the
   * Invalid progress signal for an era-expired attempt the caller retries;
   * the retry emits its own Signed/Broadcasted events.
   */
  private signAndSubmitAttempt(
    tx: PapiTransaction,
    progressCallback: ProgressCallback | undefined,
    waitFor: "in_block" | "finalized",
    chunkIndex: number | undefined,
    willRetryEraExpiry: boolean,
  ): Promise<SignSubmitResult> {
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
        resolve({
          blockHash: block.hash,
          txHash: txHash || "",
          blockNumber: block.number,
          txIndex: extractStoredIndex(events),
          events,
        })
      }

      const subscription = tx.signSubmitAndWatch(this.signer).subscribe({
        next: (ev: TxStatusEvent) => {
          if (!resolved) {
            const result = mapPapiEventToProgress(
              ev,
              txHash,
              progressCallback,
              chunkIndex,
              waitFor,
            )
            if (result.txHash) txHash = result.txHash
            if (result.finish) finish(result.finish.block, result.finish.events)
          }
          // An in_block resolution keeps the subscription alive until the tx
          // finalizes: unsubscribing stops the node-side broadcast
          // (transaction_v1_stop), and a stopped tx is not re-included if its
          // block is reorged out, losing the data and stranding any
          // follow-up tx already signed with the next nonce (it then dies at
          // its mortality boundary with AncientBirthBlock).
          if (resolved && ev.type === "finalized") cleanup()
        },
        error: (err: unknown) => {
          if (resolved) {
            cleanup()
            return
          }
          resolved = true
          cleanup()
          if (
            progressCallback &&
            !(willRetryEraExpiry && isAncientBirthBlockError(err))
          ) {
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
        },
        complete: () => {
          // PAPI can complete the Observable without a finalized/in_block
          // event (e.g. txBestBlocksState fires with found:false after a
          // reorg or node restart, causing the internal continueWith() to
          // map to rxjs.EMPTY which completes immediately). Without this
          // handler the Promise hangs until the defensive timeout fires.
          if (resolved) {
            cleanup()
            return
          }
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
        },
      })

      // Defensive timeout: PAPI handles reconnects and mortality, so this
      // should rarely fire. If it does, it likely indicates a bug. Also caps
      // the background watch of an already-resolved in_block submission.
      // Default: 7 min (above PAPI's 64-block mortality window).
      const timerId = setTimeout(() => {
        if (resolved) {
          cleanup()
          return
        }
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
   * Store data on Bulletin Chain using builder pattern
   *
   * Returns a builder that allows fluent configuration of store options.
   *
   * @param data - Data to store as `Uint8Array`
   *
   * @example
   * ```typescript
   * const result = await client
   *   .store(new TextEncoder().encode('Hello, Bulletin!'))
   *   .withCodec(CidCodec.DagPb)
   *   .withHashAlgorithm('blake2b-256')
   *   .withCallback((event) => console.log('Progress:', event))
   *   .send();
   * ```
   */
  store(data: Uint8Array): StoreBuilder {
    return new StoreBuilder(this, data)
  }

  /**
   * Store data with custom options (internal, used by builder)
   *
   * **Note**: This method is public for use by the builder but users should prefer
   * the builder pattern via `store()`.
   *
   * Automatically chunks data if it exceeds the configured threshold.
   */
  async storeWithOptions(
    data: Uint8Array,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
    chunkerConfig?: Partial<ChunkerConfig>,
  ): Promise<StoreResult> {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", ErrorCode.EMPTY_DATA)
    }

    // Best-effort authorization check before submission
    {
      const willChunk =
        !!chunkerConfig || data.length > this.config.chunkingThreshold
      const chunkSize = chunkerConfig?.chunkSize ?? this.config.defaultChunkSize
      const createManifest =
        chunkerConfig?.createManifest ?? this.config.createManifest
      const required = willChunk
        ? estimateAuthorization(data.length, chunkSize, createManifest)
        : { transactions: 1, bytes: data.length }
      await this.checkAccountAuthorization(
        required.transactions,
        required.bytes,
      )
    }

    // Decide whether to chunk based on threshold or explicit chunkerConfig
    if (chunkerConfig || data.length > this.config.chunkingThreshold) {
      // Chunked uploads use structurally fixed codecs (Raw for chunks, DagPb for manifest).
      // Reject if the user explicitly set a non-default codec — it would be silently ignored.
      const userCodec = options?.cidCodec
      if (userCodec !== undefined && userCodec !== CidCodec.Raw) {
        throw new BulletinError(
          "withCodec() cannot be used with chunked uploads. " +
            "Chunks always use Raw (0x55) and the manifest always uses DagPb (0x70).",
          ErrorCode.INVALID_CONFIG,
        )
      }

      const chunked = await this.storeChunked(
        data,
        chunkerConfig,
        options,
        progressCallback,
      )
      return {
        cid: chunked.manifestCid,
        size: data.length,
        blockNumber: undefined,
        extrinsicIndex: undefined,
        chunks: {
          chunkCids: chunked.chunkCids,
          numChunks: chunked.numChunks,
        },
      }
    } else {
      return this.storeInternalSingle(data, options, progressCallback)
    }
  }

  /**
   * Internal: Store data in a single transaction (no chunking)
   */
  private async storeInternalSingle(
    data: Uint8Array,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
  ): Promise<StoreResult> {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", ErrorCode.EMPTY_DATA)
    }

    const { cidCodec, hashAlgorithm, waitFor } = resolveStoreOptions(options)
    const { cid } = await this.preparer.prepareStore(data, options)

    try {
      const tx = this.createStoreTx(data, cidCodec, hashAlgorithm)

      const result = await this.signAndSubmitWithProgress(
        tx,
        progressCallback,
        waitFor,
      )

      return {
        cid,
        size: data.length,
        blockNumber: result.blockNumber,
        extrinsicIndex:
          "txIndex" in result
            ? (result.txIndex as number | undefined)
            : undefined,
        chunks: undefined,
      }
    } catch (error) {
      throw new BulletinError(
        `Failed to store data: ${error}`,
        ErrorCode.TRANSACTION_FAILED,
        error,
      )
    }
  }

  /**
   * Store large data with automatic chunking and manifest creation
   *
   * Handles the complete workflow:
   * 1. Chunk the data
   * 2. Calculate CIDs for each chunk
   * 3. Submit each chunk as a separate transaction
   * 4. Create and submit DAG-PB manifest (if enabled)
   * 5. Return all CIDs and receipt information
   *
   * Note: Chunk submissions are not atomic. If chunk N fails, chunks 0..N-1
   * are already stored on-chain and cannot be rolled back. The caller should
   * check the error and `chunkCids` in the thrown error's context to understand
   * what was partially uploaded.
   *
   * @param data - Data to store as `Uint8Array`
   */
  private async storeChunked(
    data: Uint8Array,
    config?: Partial<ChunkerConfig>,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
  ): Promise<ChunkedStoreResult> {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", ErrorCode.EMPTY_DATA)
    }

    const { hashAlgorithm, waitFor } = resolveStoreOptions(options)

    // Prepare all chunks and manifest (CID calculation, chunking, DAG building)
    const prepared = await this.preparer.prepareStoreChunked(
      data,
      config,
      options,
    )

    const chunkCids: CID[] = []
    const totalChunks = prepared.chunks.length

    // Submit each chunk transaction
    for (const chunk of prepared.chunks) {
      if (progressCallback) {
        progressCallback({
          type: ChunkStatus.ChunkStarted,
          index: chunk.index,
          total: totalChunks,
        })
      }

      try {
        // Chunks are always Raw codec
        const tx = this.createStoreTx(chunk.data, CidCodec.Raw, hashAlgorithm)
        await this.signAndSubmitWithProgress(
          tx,
          progressCallback,
          waitFor,
          chunk.index,
        )
        const cid = chunk.cid
        if (cid) chunkCids.push(cid)

        if (progressCallback && cid) {
          progressCallback({
            type: ChunkStatus.ChunkCompleted,
            index: chunk.index,
            total: totalChunks,
            cid,
          })
        }
      } catch (error) {
        if (progressCallback) {
          progressCallback({
            type: ChunkStatus.ChunkFailed,
            index: chunk.index,
            total: totalChunks,
            error: error as Error,
          })
        }
        // Wrap raw errors in BulletinError for consistent error handling
        if (error instanceof BulletinError) {
          throw error
        }
        throw new BulletinError(
          `Chunk ${chunk.index} processing failed: ${error instanceof Error ? error.message : String(error)}`,
          ErrorCode.CHUNK_FAILED,
          error,
        )
      }
    }

    // Submit manifest transaction if present
    let manifestCid: CID | undefined
    if (prepared.manifest) {
      if (progressCallback) {
        progressCallback({ type: ChunkStatus.ManifestStarted })
      }

      // Manifest is always DagPb codec
      const manifestTx = this.createStoreTx(
        prepared.manifest.data,
        CidCodec.DagPb,
        hashAlgorithm,
      )
      await this.signAndSubmitWithProgress(
        manifestTx,
        progressCallback,
        waitFor,
      )
      manifestCid = prepared.manifest.cid

      if (progressCallback) {
        progressCallback({
          type: ChunkStatus.ManifestCreated,
          cid: manifestCid,
        })
      }
    }

    if (progressCallback) {
      progressCallback({ type: ChunkStatus.Completed, manifestCid })
    }

    return {
      chunkCids,
      manifestCid,
      totalSize: data.length,
      numChunks: prepared.chunks.length,
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

  /** Cached renewal call-shape resolution; a rejected probe is not cached. */
  private renewShapePromise?: Promise<RenewShape>

  /**
   * Resolve which call shape the runtime's renewal extrinsics take, once per
   * client.
   *
   * On a real PAPI `TypedApi`, `tx.TransactionStorage.force_renew` is a proxy
   * entry that is truthy for *any* name, so presence alone proves nothing; the
   * entry's `getCompatibilityLevel()` compares descriptors against the live
   * runtime and returns `CompatibilityLevel.Incompatible` (0) when the runtime
   * lacks the call. Hand-rolled api objects (tests/mocks) have no such probe —
   * there, presence of `force_renew` decides.
   *
   * A probe failure throws instead of guessing — dispatching the wrong shape
   * yields an opaque encode error — and is not cached, so the next call
   * retries. A resolved shape is cached for the client's lifetime; after a
   * runtime upgrade that changes the renewal call shape, create a new client.
   */
  private resolveRenewShape(): Promise<RenewShape> {
    this.renewShapePromise ??= (async (): Promise<RenewShape> => {
      const forceRenew = this.api.tx.TransactionStorage.force_renew
      if (!forceRenew) return "legacy"
      const probe = (
        forceRenew as unknown as {
          getCompatibilityLevel?: () => Promise<number>
        }
      ).getCompatibilityLevel
      if (typeof probe !== "function") return "transactionRef"
      let level: number
      try {
        level = await probe.call(forceRenew)
      } catch (error) {
        throw new BulletinError(
          "failed to probe runtime compatibility for renew",
          ErrorCode.TRANSACTION_FAILED,
          error,
        )
      }
      return level > 0 ? "transactionRef" : "legacy"
    })()
    const resolved = this.renewShapePromise
    resolved.catch(() => {
      if (this.renewShapePromise === resolved) {
        this.renewShapePromise = undefined
      }
    })
    return resolved
  }

  /**
   * Schedule a one-shot renewal of stored data.
   *
   * The renewal fires once when the data reaches its retention boundary; it does
   * not renew synchronously. For immediate renewal use {@link forceRenew}.
   */
  renew(ref: TransactionRefInput): CallBuilder {
    return new CallBuilder(async (options) => {
      const entry = toTransactionRef(ref)
      const ts = this.api.tx.TransactionStorage
      let tx: PapiTransaction
      if ((await this.resolveRenewShape()) === "transactionRef") {
        tx = ts.renew({ entry })
      } else if (entry.type === "Position") {
        // Pre-`TransactionRef` runtimes take the position fields directly.
        tx = ts.renew(entry.value)
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
   * Immediately renew stored data, extending its retention from the current block.
   *
   * Requires a runtime that supports `force_renew`.
   */
  forceRenew(ref: TransactionRefInput): CallBuilder {
    return new CallBuilder(async (options) => {
      const ts = this.api.tx.TransactionStorage
      if (
        (await this.resolveRenewShape()) !== "transactionRef" ||
        !ts.force_renew
      ) {
        throw new BulletinError(
          "force_renew is not supported by this runtime",
          ErrorCode.UNSUPPORTED_OPERATION,
        )
      }
      const tx = ts.force_renew({ entry: toTransactionRef(ref) })
      return this.submitTx(
        tx,
        "Failed to force renew",
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
   * Store preimage-authorized content as an unsigned (bare) transaction.
   *
   * Use this for content that has been pre-authorized via `authorizePreimage()`.
   * The transaction is encoded as a bare (unsigned) extrinsic and submitted
   * via the client's `submit` function (from `PolkadotClient.submit`).
   *
   * @param data - The preauthorized content to store
   * @param options - Store options (codec, hashing algorithm, etc.)
   *
   * @example
   * ```typescript
   * import { blake2b256 } from '@polkadot-labs/hdkd-helpers';
   *
   * // First, authorize the content hash (requires sudo)
   * const data = new TextEncoder().encode('Hello, Bulletin!');
   * const hash = blake2b256(data);
   * await sudoClient.authorizePreimage(hash, BigInt(data.length));
   *
   * // Anyone can now submit without fees
   * const result = await client.store(data).sendUnsigned();
   * ```
   */
  async storeWithPreimageAuth(
    data: Uint8Array,
    options?: StoreOptions,
  ): Promise<StoreResult> {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", ErrorCode.EMPTY_DATA)
    }

    if (data.length > this.config.chunkingThreshold) {
      throw new BulletinError(
        "Chunked unsigned transactions not yet supported. Use signed transactions for large files.",
        ErrorCode.UNSUPPORTED_OPERATION,
      )
    }

    const { cidCodec, hashAlgorithm } = resolveStoreOptions(options)
    const { cid } = await this.preparer.prepareStore(data, options)

    try {
      const tx = this.createStoreTx(data, cidCodec, hashAlgorithm)
      const bareTx = await tx.getBareTx()
      const finalized = await this.submit(bareTx)

      if (!finalized.ok) {
        throw new BulletinError(
          `Transaction dispatch failed: ${JSON.stringify(finalized.dispatchError)}`,
          ErrorCode.TRANSACTION_FAILED,
        )
      }

      const storedEvent = finalized.events.find(
        (e) => e.type === "TransactionStorage" && e.value?.type === "Stored",
      )

      const extrinsicIndex =
        storedEvent?.value?.value != null &&
        typeof storedEvent.value.value === "object" &&
        "index" in storedEvent.value.value
          ? (storedEvent.value.value as { index?: number }).index
          : undefined

      return {
        cid,
        size: data.length,
        blockNumber: finalized.block.number,
        extrinsicIndex,
        chunks: undefined,
      }
    } catch (error) {
      if (error instanceof BulletinError) throw error
      throw new BulletinError(
        `Failed to store with preimage auth: ${error}`,
        ErrorCode.TRANSACTION_FAILED,
        error,
      )
    }
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
