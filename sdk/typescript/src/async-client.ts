// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

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
  CidCodec,
  type ClientConfig,
  DEFAULT_STORE_OPTIONS,
  HashAlgorithm,
  type ProgressCallback,
  type StoreOptions,
  type StoreResult,
  type WaitFor,
} from "./types.js"
import {
  hashAlgorithmCodecToEnum,
  isNonDefaultCidConfig,
  type ScaleHashingAlgorithm,
  toBytes,
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
    }): { unsubscribe(): void }
  }
  /** SCALE-encoded bare (unsigned) transaction ready for broadcasting */
  getBareTx(): Promise<string>
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
      store(args: { data: Binary | Uint8Array }): PapiTransaction
      store_with_cid_config(args: {
        cid: { codec: bigint; hashing: ScaleHashingAlgorithm }
        data: Binary | Uint8Array
      }): PapiTransaction
      authorize_account(args: {
        who: string
        transactions: number
        bytes: bigint
      }): PapiTransaction
      authorize_preimage(args: {
        content_hash: Binary | Uint8Array
        max_size: bigint
      }): PapiTransaction
      renew(args: { block: number; index: number }): PapiTransaction
      remove_expired_account_authorization(args: {
        who: string
      }): PapiTransaction
      remove_expired_preimage_authorization(args: {
        content_hash: Binary | Uint8Array
      }): PapiTransaction
      refresh_account_authorization(args: { who: string }): PapiTransaction
      refresh_preimage_authorization(args: {
        content_hash: Binary | Uint8Array
      }): PapiTransaction
    }
    Sudo?: {
      sudo(args: { call: unknown }): PapiTransaction
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
  transaction: string,
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
 * Shared interface for Bulletin clients (real and mock).
 *
 * Both `AsyncBulletinClient` and `MockBulletinClient` implement this interface.
 */
export interface BulletinClientInterface {
  /** Store data with options (used internally by StoreBuilder) */
  storeWithOptions(
    data: Binary | Uint8Array,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
    chunkerConfig?: Partial<ChunkerConfig>,
  ): Promise<StoreResult>
  /** Store preimage-authorized content as unsigned transaction */
  storeWithPreimageAuth?(
    data: Binary | Uint8Array,
    options?: StoreOptions,
  ): Promise<StoreResult>
  store(data: Binary | Uint8Array): StoreBuilder
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
}

/**
 * Builder for store operations with fluent API
 *
 * @example
 * ```typescript
 * import { Binary } from 'polkadot-api';
 *
 * const result = await client
 *   .store(Binary.fromText('Hello'))
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
    data: Binary | Uint8Array,
  ) {
    this.data = toBytes(data)
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
        "UNSUPPORTED_OPERATION",
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
 * import { getWsProvider } from 'polkadot-api/ws-provider/web';
 * import { AsyncBulletinClient } from '@bulletin/sdk';
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
   */
  constructor(
    api: BulletinTypedApi,
    signer: PolkadotSigner,
    submit: SubmitFn,
    config?: Partial<ClientConfig>,
  ) {
    this.api = api
    this.signer = signer
    this.submit = submit
    this.config = {
      defaultChunkSize: config?.defaultChunkSize ?? 1024 * 1024, // 1 MiB
      createManifest: config?.createManifest ?? true,
      chunkingThreshold: config?.chunkingThreshold ?? 2 * 1024 * 1024, // 2 MiB
    }
    this.preparer = new BulletinPreparer({
      defaultChunkSize: this.config.defaultChunkSize,
      createManifest: this.config.createManifest,
      chunkingThreshold: this.config.chunkingThreshold,
    })
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
          data: new Binary(data),
        })
      : this.api.tx.TransactionStorage.store({ data: new Binary(data) })
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
    return new Promise((resolve, reject) => {
      let resolved = false
      let txHash: string | undefined

      const finish = (
        block: { hash: string; number: number },
        events?: RuntimeEvent[],
      ) => {
        if (resolved) return
        resolved = true
        clearTimeout(timerId)
        subscription.unsubscribe()
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
          // Emit signed event when we first get a tx hash
          if (ev.txHash && !txHash) {
            txHash = ev.txHash as string
            if (progressCallback) {
              progressCallback({ type: "signed", txHash: txHash, chunkIndex })
            }
          }

          // Handle broadcasted event
          if (ev.type === "broadcasted" && progressCallback) {
            progressCallback({ type: "broadcasted", chunkIndex })
          }

          // Handle best block state
          if (ev.type === "txBestBlocksState" && ev.found && ev.block) {
            if (progressCallback) {
              progressCallback({
                type: "in_block",
                blockHash: ev.block.hash,
                blockNumber: ev.block.number,
                txIndex: ev.block.index,
                chunkIndex,
              })
            }

            if (waitFor === "in_block") {
              finish(ev.block, ev.events)
            }
          }

          // Handle finalized state
          if (ev.type === "finalized" && ev.block) {
            if (progressCallback) {
              progressCallback({
                type: "finalized",
                blockHash: ev.block.hash,
                blockNumber: ev.block.number,
                txIndex: ev.block.index,
                chunkIndex,
              })
            }

            finish(ev.block, ev.events)
          }
        },
        error: (err: unknown) => {
          if (!resolved) {
            resolved = true
            clearTimeout(timerId)
            reject(err)
          }
        },
      })

      // Timeout after 2 minutes
      const timerId = setTimeout(() => {
        if (!resolved) {
          resolved = true
          subscription.unsubscribe()
          reject(new BulletinError("Transaction timed out", "TIMEOUT"))
        }
      }, 120000)
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
        "INVALID_CONFIG",
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
    errorCode: string,
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
      throw new BulletinError(`${errorMessage}: ${error}`, errorCode, error)
    }
  }

  /**
   * Store data on Bulletin Chain using builder pattern
   *
   * Returns a builder that allows fluent configuration of store options.
   *
   * @param data - Data to store (PAPI Binary or Uint8Array)
   *
   * @example
   * ```typescript
   * import { Binary } from 'polkadot-api';
   *
   * // Using PAPI's Binary class (recommended)
   * const result = await client
   *   .store(Binary.fromText('Hello, Bulletin!'))
   *   .withCodec(CidCodec.DagPb)
   *   .withHashAlgorithm('blake2b-256')
   *   .withCallback((event) => {
   *     console.log('Progress:', event);
   *   })
   *   .send();
   *
   * // Or with Uint8Array
   * const result = await client
   *   .store(new Uint8Array([1, 2, 3]))
   *   .send();
   * ```
   */
  store(data: Binary | Uint8Array): StoreBuilder {
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
    data: Binary | Uint8Array,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
    chunkerConfig?: Partial<ChunkerConfig>,
  ): Promise<StoreResult> {
    const dataBytes = toBytes(data)
    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA")
    }

    // Decide whether to chunk based on threshold or explicit chunkerConfig
    if (chunkerConfig || dataBytes.length > this.config.chunkingThreshold) {
      // Chunked uploads use structurally fixed codecs (Raw for chunks, DagPb for manifest).
      // Reject if the user explicitly set a non-default codec — it would be silently ignored.
      const userCodec = options?.cidCodec
      if (userCodec !== undefined && userCodec !== CidCodec.Raw) {
        throw new BulletinError(
          "withCodec() cannot be used with chunked uploads. " +
            "Chunks always use Raw (0x55) and the manifest always uses DagPb (0x70).",
          "INVALID_CONFIG",
        )
      }

      const chunked = await this.storeChunked(
        dataBytes,
        chunkerConfig,
        options,
        progressCallback,
      )
      return {
        cid: chunked.manifestCid,
        size: dataBytes.length,
        blockNumber: undefined,
        extrinsicIndex: undefined,
        chunks: {
          chunkCids: chunked.chunkCids,
          numChunks: chunked.numChunks,
        },
      }
    } else {
      return this.storeInternalSingle(dataBytes, options, progressCallback)
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
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA")
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
        "TRANSACTION_FAILED",
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
   * @param data - Data to store (PAPI Binary or Uint8Array)
   */
  private async storeChunked(
    data: Binary | Uint8Array,
    config?: Partial<ChunkerConfig>,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
  ): Promise<ChunkedStoreResult> {
    const dataBytes = toBytes(data)

    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA")
    }

    const { hashAlgorithm, waitFor } = resolveStoreOptions(options)

    // Prepare all chunks and manifest (CID calculation, chunking, DAG building)
    const prepared = await this.preparer.prepareStoreChunked(
      dataBytes,
      config,
      options,
    )

    const chunkCids: CID[] = []
    const totalChunks = prepared.chunks.length

    // Submit each chunk transaction
    for (const chunk of prepared.chunks) {
      if (progressCallback) {
        progressCallback({
          type: "chunk_started",
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
            type: "chunk_completed",
            index: chunk.index,
            total: totalChunks,
            cid,
          })
        }
      } catch (error) {
        if (progressCallback) {
          progressCallback({
            type: "chunk_failed",
            index: chunk.index,
            total: totalChunks,
            error: error as Error,
          })
        }
        throw error
      }
    }

    // Submit manifest transaction if present
    let manifestCid: CID | undefined
    if (prepared.manifest) {
      if (progressCallback) {
        progressCallback({ type: "manifest_started" })
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
        progressCallback({ type: "manifest_created", cid: manifestCid })
      }
    }

    if (progressCallback) {
      progressCallback({ type: "completed", manifestCid })
    }

    return {
      chunkCids,
      manifestCid,
      totalSize: dataBytes.length,
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
        "AUTHORIZATION_FAILED",
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
        content_hash: new Binary(contentHash),
        max_size: maxSize,
      })
      return this.submitTx(
        this.maybeSudo(authTx, options?.sudo),
        "Failed to authorize preimage",
        "AUTHORIZATION_FAILED",
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
      return this.submitTx(tx, "Failed to renew", "TRANSACTION_FAILED", options)
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
        "AUTHORIZATION_FAILED",
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
          content_hash: new Binary(contentHash),
        })
      return this.submitTx(
        this.maybeSudo(authTx, options?.sudo),
        "Failed to refresh preimage authorization",
        "AUTHORIZATION_FAILED",
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
        "TRANSACTION_FAILED",
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
          content_hash: new Binary(contentHash),
        })
      return this.submitTx(
        tx,
        "Failed to remove expired preimage authorization",
        "TRANSACTION_FAILED",
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
   * const data = Binary.fromText('Hello, Bulletin!');
   * const hash = blake2b256(data.asBytes());
   * await sudoClient.authorizePreimage(hash, BigInt(data.asBytes().length));
   *
   * // Anyone can now submit without fees
   * const result = await client.store(data).sendUnsigned();
   * ```
   */
  async storeWithPreimageAuth(
    data: Binary | Uint8Array,
    options?: StoreOptions,
  ): Promise<StoreResult> {
    const dataBytes = toBytes(data)
    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA")
    }

    if (dataBytes.length > this.config.chunkingThreshold) {
      throw new BulletinError(
        "Chunked unsigned transactions not yet supported. Use signed transactions for large files.",
        "UNSUPPORTED_OPERATION",
      )
    }

    const { cidCodec, hashAlgorithm } = resolveStoreOptions(options)
    const { cid } = await this.preparer.prepareStore(dataBytes, options)

    try {
      const tx = this.createStoreTx(dataBytes, cidCodec, hashAlgorithm)
      const bareTxHex = await tx.getBareTx()
      const finalized = await this.submit(bareTxHex)

      if (!finalized.ok) {
        throw new BulletinError(
          `Transaction dispatch failed: ${JSON.stringify(finalized.dispatchError)}`,
          "TRANSACTION_FAILED",
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
        size: dataBytes.length,
        blockNumber: finalized.block.number,
        extrinsicIndex,
        chunks: undefined,
      }
    } catch (error) {
      if (error instanceof BulletinError) throw error
      throw new BulletinError(
        `Failed to store with preimage auth: ${error}`,
        "TRANSACTION_FAILED",
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
