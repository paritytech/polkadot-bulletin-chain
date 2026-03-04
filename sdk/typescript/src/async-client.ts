// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Async client with full transaction submission support
 */

import type { CID } from "multiformats/cid"
import { Binary, type PolkadotSigner } from "polkadot-api"
import { FixedSizeChunker } from "./chunker.js"
import { UnixFsDagBuilder } from "./dag.js"
import {
  BulletinError,
  type ChunkedStoreResult,
  type ChunkerConfig,
  CidCodec,
  DEFAULT_CHUNKER_CONFIG,
  DEFAULT_STORE_OPTIONS,
  HashAlgorithm,
  type ProgressCallback,
  type StoreOptions,
  type StoreResult,
  type WaitFor,
} from "./types.js"
import {
  calculateCid,
  estimateAuthorization,
  hashAlgorithmToScale,
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

/**
 * Configuration for the async Bulletin client
 */
export interface AsyncClientConfig {
  /** Default chunk size for large files (default: 1 MiB) */
  defaultChunkSize?: number
  /** Whether to create manifests for chunked uploads (default: true) */
  createManifest?: boolean
  /** Threshold for automatic chunking (default: 2 MiB) */
  chunkingThreshold?: number
  /** Wrap authorization calls in Sudo (default: false).
   * Set to true if the chain's Authorizer origin requires Sudo. */
  useSudo?: boolean
}

/**
 * Interface for clients that support store operations via the builder pattern.
 *
 * Both `AsyncBulletinClient` and `MockBulletinClient` implement this.
 */
export interface StoreExecutor {
  storeWithOptions(
    data: Binary | Uint8Array,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
  ): Promise<StoreResult>
  storeWithPreimageAuth?(
    data: Binary | Uint8Array,
    options?: StoreOptions,
  ): Promise<StoreResult>
}

/**
 * Shared interface for Bulletin clients (real and mock).
 *
 * Both `AsyncBulletinClient` and `MockBulletinClient` implement this interface.
 */
export interface BulletinClientInterface extends StoreExecutor {
  store(data: Binary | Uint8Array): StoreBuilder
  authorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
    cb?: ProgressCallback,
  ): Promise<TransactionReceipt>
  authorizePreimage(
    contentHash: Uint8Array,
    maxSize: bigint,
    cb?: ProgressCallback,
  ): Promise<TransactionReceipt>
  renew(
    block: number,
    index: number,
    cb?: ProgressCallback,
  ): Promise<TransactionReceipt>
  refreshAccountAuthorization(
    who: string,
    cb?: ProgressCallback,
  ): Promise<TransactionReceipt>
  refreshPreimageAuthorization(
    contentHash: Uint8Array,
    cb?: ProgressCallback,
  ): Promise<TransactionReceipt>
  removeExpiredAccountAuthorization(
    who: string,
    cb?: ProgressCallback,
  ): Promise<TransactionReceipt>
  removeExpiredPreimageAuthorization(
    contentHash: Uint8Array,
    cb?: ProgressCallback,
  ): Promise<TransactionReceipt>
  estimateAuthorization(dataSize: number): {
    transactions: number
    bytes: number
  }
  withAccount(account: string): this
  getAccount(): string | undefined
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

  constructor(
    private executor: StoreExecutor,
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
  withFinalization(waitFor: WaitFor): this {
    this.options.waitFor = waitFor
    return this
  }

  /** Set custom store options */
  withOptions(options: StoreOptions): this {
    this.options = options
    return this
  }

  /** Set progress callback for chunked uploads */
  withCallback(callback: ProgressCallback): this {
    this.callback = callback
    return this
  }

  /** Execute the store operation (signed transaction, uses account authorization) */
  async send(): Promise<StoreResult> {
    return this.executor.storeWithOptions(
      this.data,
      this.options,
      this.callback,
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

/** Convert Binary or Uint8Array to Uint8Array */
function toBytes(data: Binary | Uint8Array): Uint8Array {
  return data instanceof Uint8Array ? data : data.asBytes()
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
    waitFor: opts.waitFor ?? "best_block",
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
  public config: Required<AsyncClientConfig>
  /** Account for authorization checks (optional) */
  private account?: string

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
    config?: Partial<AsyncClientConfig>,
  ) {
    this.api = api
    this.signer = signer
    this.submit = submit
    this.config = {
      defaultChunkSize: config?.defaultChunkSize ?? 1024 * 1024, // 1 MiB
      createManifest: config?.createManifest ?? true,
      chunkingThreshold: config?.chunkingThreshold ?? 2 * 1024 * 1024, // 2 MiB
      useSudo: config?.useSudo ?? false,
    }
  }

  /**
   * Set the account for authorization checks
   */
  withAccount(account: string): this {
    this.account = account
    return this
  }

  /**
   * Get the account set for authorization checks
   */
  getAccount(): string | undefined {
    return this.account
  }

  /**
   * Create a store transaction, using store_with_cid_config when non-default CID settings are used.
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
            hashing: hashAlgorithmToScale(hashAlgorithm),
          },
          data: new Binary(data),
        })
      : this.api.tx.TransactionStorage.store({ data: new Binary(data) })
  }

  /**
   * Sign, submit, and wait for a transaction to be finalized.
   *
   * Uses PAPI's signAndSubmit which returns a promise resolving to the
   * finalized result directly.
   */
  private async signAndSubmitFinalized(tx: PapiTransaction): Promise<{
    blockHash: string
    txHash: string
    blockNumber?: number
    events?: RuntimeEvent[]
  }> {
    const result = await tx.signAndSubmit(this.signer)
    return {
      blockHash: result.block?.hash ?? "",
      txHash: result.txHash,
      blockNumber: result.block?.number,
      events: result.events,
    }
  }

  /**
   * Sign, submit, and watch a transaction with progress callbacks.
   *
   * Uses PAPI's signSubmitAndWatch which provides real-time status updates
   * as the transaction progresses through the network.
   *
   * @param tx - The transaction to submit
   * @param progressCallback - Optional callback to receive transaction status events
   * @param waitFor - What to wait for: "best_block" (faster) or "finalized" (safer, default)
   */
  private async signAndSubmitWithProgress(
    tx: PapiTransaction,
    progressCallback?: ProgressCallback,
    waitFor: "best_block" | "finalized" = "finalized",
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
              progressCallback({ type: "signed", txHash: txHash })
            }
          }

          // Handle broadcasted event
          if (ev.type === "broadcasted" && progressCallback) {
            progressCallback({ type: "broadcasted" })
          }

          // Handle best block state
          if (ev.type === "txBestBlocksState" && ev.found && ev.block) {
            if (progressCallback) {
              progressCallback({
                type: "best_block",
                blockHash: ev.block.hash,
                blockNumber: ev.block.number,
                txIndex: ev.block.index,
              })
            }

            if (waitFor === "best_block") {
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
              })
            }

            finish(ev.block, ev.events)
          }
        },
        error: (err: unknown) => {
          if (!resolved) {
            resolved = true
            reject(err)
          }
        },
      })

      // Timeout after 2 minutes
      setTimeout(() => {
        if (!resolved) {
          resolved = true
          subscription.unsubscribe()
          reject(new BulletinError("Transaction timed out", "TIMEOUT"))
        }
      }, 120000)
    })
  }

  /**
   * Wrap a call in Sudo if configured, otherwise return it as a direct transaction
   */
  private wrapInSudo(tx: PapiTransaction): PapiTransaction {
    if (!this.config.useSudo) return tx
    if (!this.api.tx.Sudo) {
      throw new BulletinError(
        "useSudo is enabled but Sudo pallet is not available on this chain",
        "INVALID_CONFIG",
      )
    }
    return this.api.tx.Sudo.sudo({ call: tx.decodedCall })
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
  ): Promise<StoreResult> {
    const dataBytes = toBytes(data)
    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA")
    }

    // Decide whether to chunk based on threshold
    if (dataBytes.length > this.config.chunkingThreshold) {
      const chunked = await this.storeChunked(
        dataBytes,
        undefined,
        options,
        progressCallback,
      )
      const primaryCid = chunked.manifestCid ?? chunked.chunkCids[0]
      if (!primaryCid) {
        throw new BulletinError(
          "No CID produced from chunked upload",
          "CID_CALCULATION_FAILED",
        )
      }
      return {
        cid: primaryCid,
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
    const cid = await calculateCid(data, cidCodec, hashAlgorithm)

    try {
      const tx = this.createStoreTx(data, cidCodec, hashAlgorithm)

      // Use progress-aware submission if callback provided, otherwise use simple submission
      const result = progressCallback
        ? await this.signAndSubmitWithProgress(tx, progressCallback, waitFor)
        : await this.signAndSubmitFinalized(tx)

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
   * @param data - Data to store (PAPI Binary or Uint8Array)
   */
  async storeChunked(
    data: Binary | Uint8Array,
    config?: Partial<ChunkerConfig>,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
  ): Promise<ChunkedStoreResult> {
    const dataBytes = toBytes(data)

    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA")
    }

    const chunkerConfig: ChunkerConfig = {
      ...DEFAULT_CHUNKER_CONFIG,
      chunkSize: config?.chunkSize ?? this.config.defaultChunkSize,
      createManifest: config?.createManifest ?? this.config.createManifest,
    }

    const { cidCodec, hashAlgorithm } = resolveStoreOptions(options)

    // Chunk the data
    const chunker = new FixedSizeChunker(chunkerConfig)
    const chunks = chunker.chunk(dataBytes)

    const chunkCids: CID[] = []

    // Submit each chunk
    for (const chunk of chunks) {
      if (progressCallback) {
        progressCallback({
          type: "chunk_started",
          index: chunk.index,
          total: chunks.length,
        })
      }

      try {
        const cid = await calculateCid(chunk.data, cidCodec, hashAlgorithm)

        chunk.cid = cid

        const tx = this.createStoreTx(chunk.data, cidCodec, hashAlgorithm)
        await this.signAndSubmitFinalized(tx)

        chunkCids.push(cid)

        if (progressCallback) {
          progressCallback({
            type: "chunk_completed",
            index: chunk.index,
            total: chunks.length,
            cid,
          })
        }
      } catch (error) {
        if (progressCallback) {
          progressCallback({
            type: "chunk_failed",
            index: chunk.index,
            total: chunks.length,
            error: error as Error,
          })
        }
        if (error instanceof BulletinError) {
          throw error
        }
        throw new BulletinError(
          `Chunk ${chunk.index} processing failed: ${error instanceof Error ? error.message : String(error)}`,
          "CHUNK_FAILED",
          error,
        )
      }
    }

    // Optionally create and submit manifest
    let manifestCid: CID | undefined
    if (chunkerConfig.createManifest) {
      if (progressCallback) {
        progressCallback({ type: "manifest_started" })
      }

      const builder = new UnixFsDagBuilder()
      const manifest = await builder.build(chunks, hashAlgorithm)

      const manifestTx = this.createStoreTx(
        manifest.dagBytes,
        cidCodec,
        hashAlgorithm,
      )
      await this.signAndSubmitFinalized(manifestTx)

      manifestCid = manifest.rootCid

      if (progressCallback) {
        progressCallback({
          type: "manifest_created",
          cid: manifest.rootCid,
        })
      }
    }

    if (progressCallback) {
      progressCallback({
        type: "completed",
        manifestCid,
      })
    }

    return {
      chunkCids,
      manifestCid,
      totalSize: dataBytes.length,
      numChunks: chunks.length,
    }
  }

  /**
   * Authorize an account to store data
   *
   * Wraps in Sudo if `useSudo: true` is set in config (default: false).
   *
   * @param who - Account address to authorize
   * @param transactions - Number of transactions to authorize
   * @param bytes - Maximum bytes to authorize
   * @param progressCallback - Optional callback to receive transaction status events
   */
  async authorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
    progressCallback?: ProgressCallback,
  ): Promise<TransactionReceipt> {
    try {
      const authTx = this.api.tx.TransactionStorage.authorize_account({
        who,
        transactions,
        bytes,
      })
      const tx = this.wrapInSudo(authTx)

      const result = progressCallback
        ? await this.signAndSubmitWithProgress(tx, progressCallback)
        : await this.signAndSubmitFinalized(tx)

      return {
        blockHash: result.blockHash,
        txHash: result.txHash,
        blockNumber: result.blockNumber,
      }
    } catch (error) {
      throw new BulletinError(
        `Failed to authorize account: ${error}`,
        "AUTHORIZATION_FAILED",
        error,
      )
    }
  }

  /**
   * Authorize a preimage (by content hash) to be stored
   *
   * Wraps in Sudo if `useSudo: true` is set in config (default: false).
   *
   * @param contentHash - Blake2b-256 hash of the content to authorize
   * @param maxSize - Maximum size in bytes for the content
   * @param progressCallback - Optional callback to receive transaction status events
   */
  async authorizePreimage(
    contentHash: Uint8Array,
    maxSize: bigint,
    progressCallback?: ProgressCallback,
  ): Promise<TransactionReceipt> {
    try {
      const authTx = this.api.tx.TransactionStorage.authorize_preimage({
        content_hash: new Binary(contentHash),
        max_size: maxSize,
      })
      const tx = this.wrapInSudo(authTx)

      const result = progressCallback
        ? await this.signAndSubmitWithProgress(tx, progressCallback)
        : await this.signAndSubmitFinalized(tx)

      return {
        blockHash: result.blockHash,
        txHash: result.txHash,
        blockNumber: result.blockNumber,
      }
    } catch (error) {
      throw new BulletinError(
        `Failed to authorize preimage: ${error}`,
        "AUTHORIZATION_FAILED",
        error,
      )
    }
  }

  /**
   * Renew/extend retention period for stored data
   *
   * @param block - Block number where the original storage transaction was included
   * @param index - Extrinsic index within the block
   * @param progressCallback - Optional callback to receive transaction status events
   */
  async renew(
    block: number,
    index: number,
    progressCallback?: ProgressCallback,
  ): Promise<TransactionReceipt> {
    try {
      const tx = this.api.tx.TransactionStorage.renew({ block, index })

      const result = progressCallback
        ? await this.signAndSubmitWithProgress(tx, progressCallback)
        : await this.signAndSubmitFinalized(tx)

      return {
        blockHash: result.blockHash,
        txHash: result.txHash,
        blockNumber: result.blockNumber,
      }
    } catch (error) {
      throw new BulletinError(
        `Failed to renew: ${error}`,
        "TRANSACTION_FAILED",
        error,
      )
    }
  }

  /**
   * Refresh an account authorization (extends expiry)
   *
   * Wraps in Sudo if `useSudo: true` is set in config (default: false).
   * Requires Authorizer origin on-chain.
   *
   * @param who - Account address to refresh authorization for
   * @param progressCallback - Optional callback to receive transaction status events
   */
  async refreshAccountAuthorization(
    who: string,
    progressCallback?: ProgressCallback,
  ): Promise<TransactionReceipt> {
    try {
      const authTx =
        this.api.tx.TransactionStorage.refresh_account_authorization({ who })
      const tx = this.wrapInSudo(authTx)

      const result = progressCallback
        ? await this.signAndSubmitWithProgress(tx, progressCallback)
        : await this.signAndSubmitFinalized(tx)

      return {
        blockHash: result.blockHash,
        txHash: result.txHash,
        blockNumber: result.blockNumber,
      }
    } catch (error) {
      throw new BulletinError(
        `Failed to refresh account authorization: ${error}`,
        "AUTHORIZATION_FAILED",
        error,
      )
    }
  }

  /**
   * Refresh a preimage authorization (extends expiry)
   *
   * Wraps in Sudo if `useSudo: true` is set in config (default: false).
   * Requires Authorizer origin on-chain.
   *
   * @param contentHash - Blake2b-256 hash of the authorized content
   * @param progressCallback - Optional callback to receive transaction status events
   */
  async refreshPreimageAuthorization(
    contentHash: Uint8Array,
    progressCallback?: ProgressCallback,
  ): Promise<TransactionReceipt> {
    try {
      const authTx =
        this.api.tx.TransactionStorage.refresh_preimage_authorization({
          content_hash: new Binary(contentHash),
        })
      const tx = this.wrapInSudo(authTx)

      const result = progressCallback
        ? await this.signAndSubmitWithProgress(tx, progressCallback)
        : await this.signAndSubmitFinalized(tx)

      return {
        blockHash: result.blockHash,
        txHash: result.txHash,
        blockNumber: result.blockNumber,
      }
    } catch (error) {
      throw new BulletinError(
        `Failed to refresh preimage authorization: ${error}`,
        "AUTHORIZATION_FAILED",
        error,
      )
    }
  }

  /**
   * Remove an expired account authorization
   *
   * Can be called by anyone (no special origin required).
   *
   * @param who - Account address with expired authorization
   * @param progressCallback - Optional callback to receive transaction status events
   */
  async removeExpiredAccountAuthorization(
    who: string,
    progressCallback?: ProgressCallback,
  ): Promise<TransactionReceipt> {
    try {
      const tx =
        this.api.tx.TransactionStorage.remove_expired_account_authorization({
          who,
        })

      const result = progressCallback
        ? await this.signAndSubmitWithProgress(tx, progressCallback)
        : await this.signAndSubmitFinalized(tx)

      return {
        blockHash: result.blockHash,
        txHash: result.txHash,
        blockNumber: result.blockNumber,
      }
    } catch (error) {
      throw new BulletinError(
        `Failed to remove expired account authorization: ${error}`,
        "TRANSACTION_FAILED",
        error,
      )
    }
  }

  /**
   * Remove an expired preimage authorization
   *
   * Can be called by anyone (no special origin required).
   *
   * @param contentHash - Blake2b-256 hash of the expired authorization
   * @param progressCallback - Optional callback to receive transaction status events
   */
  async removeExpiredPreimageAuthorization(
    contentHash: Uint8Array,
    progressCallback?: ProgressCallback,
  ): Promise<TransactionReceipt> {
    try {
      const tx =
        this.api.tx.TransactionStorage.remove_expired_preimage_authorization({
          content_hash: new Binary(contentHash),
        })

      const result = progressCallback
        ? await this.signAndSubmitWithProgress(tx, progressCallback)
        : await this.signAndSubmitFinalized(tx)

      return {
        blockHash: result.blockHash,
        txHash: result.txHash,
        blockNumber: result.blockNumber,
      }
    } catch (error) {
      throw new BulletinError(
        `Failed to remove expired preimage authorization: ${error}`,
        "TRANSACTION_FAILED",
        error,
      )
    }
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
    const cid = await calculateCid(dataBytes, cidCodec, hashAlgorithm)

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
    return estimateAuthorization(
      dataSize,
      this.config.defaultChunkSize,
      this.config.createManifest,
    )
  }
}
