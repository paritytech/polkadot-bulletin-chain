// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Async client with full transaction submission support
 */

import { CID } from "multiformats/cid";
import { PolkadotSigner, TypedApi, Binary } from "polkadot-api";
import { FixedSizeChunker, reassembleChunks } from "./chunker.js";
import { UnixFsDagBuilder } from "./dag.js";
import { calculateCid } from "./utils.js";
import {
  ClientConfig,
  StoreOptions,
  DEFAULT_STORE_OPTIONS,
  ChunkerConfig,
  DEFAULT_CHUNKER_CONFIG,
  ChunkedStoreResult,
  StoreResult,
  ProgressCallback,
  BulletinError,
  CidCodec,
  HashAlgorithm,
  ChunkDetails,
  Authorization,
  WaitFor,
} from "./types.js";

/**
 * Transaction receipt from a successful submission
 */
export interface TransactionReceipt {
  /** Block hash containing the transaction */
  blockHash: string;
  /** Transaction hash */
  txHash: string;
  /** Block number (if known) */
  blockNumber?: number;
}

/**
 * Configuration for the async Bulletin client
 */
export interface AsyncClientConfig {
  /** Default chunk size for large files (default: 1 MiB) */
  defaultChunkSize?: number;
  /** Maximum parallel uploads (default: 8) */
  maxParallel?: number;
  /** Whether to create manifests for chunked uploads (default: true) */
  createManifest?: boolean;
  /** Threshold for automatic chunking (default: 2 MiB) */
  chunkingThreshold?: number;
  /** Check authorization before uploading to fail fast (default: true) */
  checkAuthorizationBeforeUpload?: boolean;
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
  private data: Uint8Array;
  private options: StoreOptions = { ...DEFAULT_STORE_OPTIONS };
  private callback?: ProgressCallback;

  constructor(
    private client: AsyncBulletinClient,
    data: Binary | Uint8Array,
  ) {
    // Convert Binary to Uint8Array if needed
    this.data = data instanceof Uint8Array ? data : data.asBytes();
  }

  /** Set the CID codec */
  withCodec(codec: CidCodec): this {
    this.options.cidCodec = codec;
    return this;
  }

  /** Set the hash algorithm */
  withHashAlgorithm(algorithm: HashAlgorithm): this {
    this.options.hashingAlgorithm = algorithm;
    return this;
  }

  /** Set whether to wait for finalization */
  withFinalization(wait: boolean): this {
    this.options.waitForFinalization = wait;
    return this;
  }

  /** Set custom store options */
  withOptions(options: StoreOptions): this {
    this.options = options;
    return this;
  }

  /** Set progress callback for chunked uploads */
  withCallback(callback: ProgressCallback): this {
    this.callback = callback;
    return this;
  }

  /** Execute the store operation (signed transaction, uses account authorization) */
  async send(): Promise<StoreResult> {
    return this.client.storeWithOptions(this.data, this.options, this.callback);
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
    return this.client.storeWithPreimageAuth(
      this.data,
      this.options,
      this.callback,
    );
  }
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
 * const bulletinClient = new AsyncBulletinClient(api, signer);
 *
 * // Store data
 * const result = await bulletinClient.store(data).send();
 * ```
 */
export class AsyncBulletinClient {
  /** PAPI client for blockchain interaction */
  public api: any;
  /** Signer for transaction signing */
  public signer: PolkadotSigner;
  /** Client configuration */
  public config: Required<AsyncClientConfig>;
  /** Account for authorization checks (optional) */
  private account?: string;

  /**
   * Create a new async client with PAPI client and signer
   *
   * The PAPI client must be configured with the correct chain metadata
   * for your Bulletin Chain node.
   *
   * @param api - Configured PAPI TypedApi instance
   * @param signer - Polkadot signer for transaction signing
   * @param config - Optional client configuration
   */
  constructor(
    api: any,
    signer: PolkadotSigner,
    config?: Partial<AsyncClientConfig>,
  ) {
    this.api = api;
    this.signer = signer;
    this.config = {
      defaultChunkSize: config?.defaultChunkSize ?? 1024 * 1024, // 1 MiB
      maxParallel: config?.maxParallel ?? 8,
      createManifest: config?.createManifest ?? true,
      chunkingThreshold: config?.chunkingThreshold ?? 2 * 1024 * 1024, // 2 MiB
      checkAuthorizationBeforeUpload:
        config?.checkAuthorizationBeforeUpload ?? true,
    };
  }

  /**
   * Set the account for authorization checks
   *
   * If set and `checkAuthorizationBeforeUpload` is enabled, the client will
   * query authorization state before uploading and fail fast if insufficient.
   */
  withAccount(account: string): this {
    this.account = account;
    return this;
  }

  /**
   * Sign, submit, and wait for a transaction to be finalized.
   *
   * Uses PAPI's signAndSubmit which returns a promise resolving to the
   * finalized result directly.
   */
  private async signAndSubmitFinalized(tx: any): Promise<{
    blockHash: string;
    txHash: string;
    blockNumber?: number;
    events?: any[];
  }> {
    const result = await tx.signAndSubmit(this.signer);
    return {
      blockHash: result.block?.hash,
      txHash: result.txHash,
      blockNumber: result.block?.number,
      events: result.events,
    };
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
    tx: any,
    progressCallback?: ProgressCallback,
    waitFor: "best_block" | "finalized" = "finalized",
  ): Promise<{
    blockHash: string;
    txHash: string;
    blockNumber?: number;
    txIndex?: number;
    events?: any[];
  }> {
    return new Promise((resolve, reject) => {
      let resolved = false;
      let txHash: string | undefined;

      const subscription = tx.signSubmitAndWatch(this.signer).subscribe({
        next: (ev: any) => {
          // Emit signed event when we first get a tx hash
          if (ev.txHash && !txHash) {
            txHash = ev.txHash as string;
            if (progressCallback) {
              progressCallback({ type: "signed", txHash: txHash });
            }
          }

          // Handle broadcasted event
          if (ev.type === "broadcasted" && progressCallback) {
            progressCallback({ type: "broadcasted" });
          }

          // Handle best block state
          if (ev.type === "txBestBlocksState" && ev.found) {
            if (progressCallback) {
              progressCallback({
                type: "best_block",
                blockHash: ev.block.hash,
                blockNumber: ev.block.number,
                txIndex: ev.block.index,
              });
            }

            // If waiting for best_block, resolve here
            if (waitFor === "best_block" && !resolved) {
              resolved = true;
              subscription.unsubscribe();

              // Extract tx index from Stored event if available
              let storedIndex: number | undefined;
              if (ev.events) {
                const storedEvent = ev.events.find(
                  (e: any) =>
                    e.type === "TransactionStorage" && e.value?.type === "Stored",
                );
                if (storedEvent?.value?.value?.index !== undefined) {
                  storedIndex = storedEvent.value.value.index;
                }
              }

              resolve({
                blockHash: ev.block.hash,
                txHash: txHash || "",
                blockNumber: ev.block.number,
                txIndex: storedIndex,
                events: ev.events,
              });
            }
          }

          // Handle finalized state
          if (ev.type === "finalized") {
            if (progressCallback) {
              progressCallback({
                type: "finalized",
                blockHash: ev.block.hash,
                blockNumber: ev.block.number,
                txIndex: ev.block.index,
              });
            }

            if (!resolved) {
              resolved = true;
              subscription.unsubscribe();

              // Extract tx index from Stored event if available
              let storedIndex: number | undefined;
              if (ev.events) {
                const storedEvent = ev.events.find(
                  (e: any) =>
                    e.type === "TransactionStorage" && e.value?.type === "Stored",
                );
                if (storedEvent?.value?.value?.index !== undefined) {
                  storedIndex = storedEvent.value.value.index;
                }
              }

              resolve({
                blockHash: ev.block.hash,
                txHash: txHash || "",
                blockNumber: ev.block.number,
                txIndex: storedIndex,
                events: ev.events,
              });
            }
          }
        },
        error: (err: any) => {
          if (!resolved) {
            resolved = true;
            reject(err);
          }
        },
      });

      // Timeout after 2 minutes
      setTimeout(() => {
        if (!resolved) {
          resolved = true;
          subscription.unsubscribe();
          reject(new BulletinError("Transaction timed out", "TIMEOUT"));
        }
      }, 120000);
    });
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
    return new StoreBuilder(this, data);
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
    // Convert Binary to Uint8Array if needed
    const dataBytes = data instanceof Uint8Array ? data : data.asBytes();
    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }

    // Decide whether to chunk based on threshold
    if (dataBytes.length > this.config.chunkingThreshold) {
      // Large data - use chunking
      return this.storeInternalChunked(
        dataBytes,
        undefined,
        options,
        progressCallback,
      );
    } else {
      // Small data - single transaction
      return this.storeInternalSingle(dataBytes, options, progressCallback);
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
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }

    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };

    // Calculate CID using defaults if not specified
    const cidCodec = opts.cidCodec ?? CidCodec.Raw;
    const hashAlgorithm = opts.hashingAlgorithm ?? DEFAULT_STORE_OPTIONS.hashingAlgorithm;

    // Determine wait strategy (support both old and new options)
    const waitFor: WaitFor = opts.waitFor ??
      (opts.waitForFinalization ? "finalized" : "best_block");

    const cid = await calculateCid(data, cidCodec, hashAlgorithm);

    try {
      const tx = this.api.tx.TransactionStorage.store({
        data: new Binary(data),
      });

      // Use progress-aware submission if callback provided, otherwise use simple submission
      const result = progressCallback
        ? await this.signAndSubmitWithProgress(tx, progressCallback, waitFor)
        : await this.signAndSubmitFinalized(tx);

      return {
        cid,
        size: data.length,
        blockNumber: result.blockNumber,
        extrinsicIndex: "txIndex" in result ? (result.txIndex as number | undefined) : undefined,
        chunks: undefined,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to store data: ${error}`,
        "TRANSACTION_FAILED",
        error,
      );
    }
  }

  /**
   * Internal: Store data with chunking
   */
  private async storeInternalChunked(
    data: Uint8Array,
    config?: Partial<ChunkerConfig>,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
  ): Promise<StoreResult> {
    const chunkerConfig: ChunkerConfig = {
      ...DEFAULT_CHUNKER_CONFIG,
      chunkSize: config?.chunkSize ?? this.config.defaultChunkSize,
      maxParallel: config?.maxParallel ?? this.config.maxParallel,
      createManifest: config?.createManifest ?? this.config.createManifest,
    };

    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };

    // Chunk the data
    const chunker = new FixedSizeChunker(chunkerConfig);
    const chunks = chunker.chunk(data);

    const chunkCids: CID[] = [];

    // Submit each chunk sequentially
    for (const chunk of chunks) {
      if (progressCallback) {
        progressCallback({
          type: "chunk_started",
          index: chunk.index,
          total: chunks.length,
        });
      }

      try {
        const cid = await calculateCid(
          chunk.data,
          opts.cidCodec ?? CidCodec.Raw,
          opts.hashingAlgorithm!,
        );
        chunk.cid = cid;

        const tx = this.api.tx.TransactionStorage.store({
          data: new Binary(chunk.data),
        });
        await this.signAndSubmitFinalized(tx);

        chunkCids.push(cid);

        if (progressCallback) {
          progressCallback({
            type: "chunk_completed",
            index: chunk.index,
            total: chunks.length,
            cid,
          });
        }
      } catch (error) {
        if (progressCallback) {
          progressCallback({
            type: "chunk_failed",
            index: chunk.index,
            total: chunks.length,
            error: error as Error,
          });
        }
        throw error;
      }
    }

    // Optionally create and submit manifest
    let manifestCid: CID | undefined;
    if (chunkerConfig.createManifest) {
      if (progressCallback) {
        progressCallback({ type: "manifest_started" });
      }

      const builder = new UnixFsDagBuilder();
      const manifest = await builder.build(chunks, opts.hashingAlgorithm!);

      const manifestTx = this.api.tx.TransactionStorage.store({
        data: new Binary(manifest.dagBytes),
      });
      await this.signAndSubmitFinalized(manifestTx);

      manifestCid = manifest.rootCid;

      if (progressCallback) {
        progressCallback({
          type: "manifest_created",
          cid: manifest.rootCid,
        });
      }
    }

    if (progressCallback) {
      progressCallback({ type: "completed", manifestCid });
    }

    const primaryCid = manifestCid ?? chunkCids[0];

    return {
      cid: primaryCid,
      size: data.length,
      blockNumber: undefined,
      extrinsicIndex: undefined,
      chunks: {
        chunkCids,
        numChunks: chunks.length,
      },
    };
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
    // Convert Binary to Uint8Array if needed
    const dataBytes = data instanceof Uint8Array ? data : data.asBytes();

    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }

    const chunkerConfig: ChunkerConfig = {
      ...DEFAULT_CHUNKER_CONFIG,
      chunkSize: config?.chunkSize ?? this.config.defaultChunkSize,
      maxParallel: config?.maxParallel ?? this.config.maxParallel,
      createManifest: config?.createManifest ?? this.config.createManifest,
    };

    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };

    // Extract options with defaults
    const cidCodec = opts.cidCodec ?? CidCodec.Raw;
    const hashAlgorithm = opts.hashingAlgorithm ?? DEFAULT_STORE_OPTIONS.hashingAlgorithm;

    // Chunk the data
    const chunker = new FixedSizeChunker(chunkerConfig);
    const chunks = chunker.chunk(dataBytes);

    const chunkCids: CID[] = [];

    // Submit each chunk
    for (const chunk of chunks) {
      if (progressCallback) {
        progressCallback({
          type: "chunk_started",
          index: chunk.index,
          total: chunks.length,
        });
      }

      try {
        // Calculate CID for this chunk
        const cid = await calculateCid(chunk.data, cidCodec, hashAlgorithm);

        chunk.cid = cid;

        const tx = this.api.tx.TransactionStorage.store({
          data: new Binary(chunk.data),
        });
        await this.signAndSubmitFinalized(tx);

        chunkCids.push(cid);

        if (progressCallback) {
          progressCallback({
            type: "chunk_completed",
            index: chunk.index,
            total: chunks.length,
            cid,
          });
        }
      } catch (error) {
        if (progressCallback) {
          progressCallback({
            type: "chunk_failed",
            index: chunk.index,
            total: chunks.length,
            error: error as Error,
          });
        }
        // Wrap raw errors in BulletinError for consistent error handling
        if (error instanceof BulletinError) {
          throw error;
        }
        throw new BulletinError(
          `Chunk ${chunk.index} processing failed: ${error instanceof Error ? error.message : String(error)}`,
          "CHUNK_FAILED",
          error,
        );
      }
    }

    // Optionally create and submit manifest
    let manifestCid: CID | undefined;
    if (chunkerConfig.createManifest) {
      if (progressCallback) {
        progressCallback({ type: "manifest_started" });
      }

      const builder = new UnixFsDagBuilder();
      const manifest = await builder.build(chunks, hashAlgorithm);

      const manifestTx = this.api.tx.TransactionStorage.store({
        data: new Binary(manifest.dagBytes),
      });
      await this.signAndSubmitFinalized(manifestTx);

      manifestCid = manifest.rootCid;

      if (progressCallback) {
        progressCallback({
          type: "manifest_created",
          cid: manifest.rootCid,
        });
      }
    }

    if (progressCallback) {
      progressCallback({
        type: "completed",
        manifestCid,
      });
    }

    return {
      chunkCids,
      manifestCid,
      totalSize: dataBytes.length,
      numChunks: chunks.length,
    };
  }

  /**
   * Authorize an account to store data
   *
   * Requires sudo/authorizer privileges
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
      const authCall = this.api.tx.TransactionStorage.authorize_account({
        who,
        transactions,
        bytes,
      }).decodedCall;

      const sudoTx = this.api.tx.Sudo.sudo({ call: authCall });

      // Use progress-aware submission if callback provided
      const result = progressCallback
        ? await this.signAndSubmitWithProgress(sudoTx, progressCallback)
        : await this.signAndSubmitFinalized(sudoTx);

      return {
        blockHash: result.blockHash,
        txHash: result.txHash,
        blockNumber: result.blockNumber,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to authorize account: ${error}`,
        "AUTHORIZATION_FAILED",
        error,
      );
    }
  }

  /**
   * Authorize a preimage (by content hash) to be stored
   *
   * Requires sudo/authorizer privileges
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
      const authCall = this.api.tx.TransactionStorage.authorize_preimage({
        content_hash: contentHash,
        max_size: maxSize,
      }).decodedCall;

      const sudoTx = this.api.tx.Sudo.sudo({ call: authCall });

      // Use progress-aware submission if callback provided
      const result = progressCallback
        ? await this.signAndSubmitWithProgress(sudoTx, progressCallback)
        : await this.signAndSubmitFinalized(sudoTx);

      return {
        blockHash: result.blockHash,
        txHash: result.txHash,
        blockNumber: result.blockNumber,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to authorize preimage: ${error}`,
        "AUTHORIZATION_FAILED",
        error,
      );
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
      const tx = this.api.tx.TransactionStorage.renew({ block, index });

      // Use progress-aware submission if callback provided
      const result = progressCallback
        ? await this.signAndSubmitWithProgress(tx, progressCallback)
        : await this.signAndSubmitFinalized(tx);

      return {
        blockHash: result.blockHash,
        txHash: result.txHash,
        blockNumber: result.blockNumber,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to renew: ${error}`,
        "TRANSACTION_FAILED",
        error,
      );
    }
  }

  /**
   * Store preimage-authorized content as unsigned transaction
   *
   * Use this for content that has been pre-authorized via `authorizePreimage()`.
   * Unsigned transactions don't require fees and can be submitted by anyone who
   * has the preauthorized content.
   *
   * @param data - The preauthorized content to store
   * @param options - Store options (codec, hashing algorithm, etc.)
   * @param progressCallback - Optional progress callback for chunked uploads
   *
   * @example
   * ```typescript
   * import { blake2b256 } from '@noble/hashes/blake2b';
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
    progressCallback?: ProgressCallback,
  ): Promise<StoreResult> {
    // Convert Binary to Uint8Array if needed
    const dataBytes = data instanceof Uint8Array ? data : data.asBytes();
    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }

    // For now, only support single-chunk unsigned transactions
    // Chunked unsigned transactions would require submitting multiple unsigned txs
    if (dataBytes.length > this.config.chunkingThreshold) {
      throw new BulletinError(
        "Chunked unsigned transactions not yet supported. Use signed transactions for large files.",
        "UNSUPPORTED_OPERATION",
      );
    }

    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };

    // Calculate CID using defaults if not specified
    const cidCodec = opts.cidCodec ?? CidCodec.Raw;
    const hashAlgorithm = opts.hashingAlgorithm ?? DEFAULT_STORE_OPTIONS.hashingAlgorithm;

    const cid = await calculateCid(dataBytes, cidCodec, hashAlgorithm);

    try {
      // Submit as unsigned transaction
      // PAPI's unsigned transaction API: create tx without signer, then submit
      const tx = this.api.tx.TransactionStorage.store({ data: dataBytes });

      // For unsigned transactions, PAPI requires submitting without calling signAndSubmit
      // Instead, we need to use the raw submission API
      // Note: The exact API depends on PAPI version, this may need adjustment
      const result = await tx.submit();

      // Wait for finalization
      const finalized = await result.waitFor("finalized");

      // Extract extrinsic index from Stored event
      const storedEvent = finalized.events.find(
        (e: any) =>
          e.type === "TransactionStorage" && e.value.type === "Stored",
      );

      const extrinsicIndex = storedEvent?.value.value?.index;
      const blockNumber = finalized.blockNumber;

      return {
        cid,
        size: dataBytes.length,
        blockNumber,
        extrinsicIndex,
        chunks: undefined,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to store with preimage auth: ${error}`,
        "TRANSACTION_FAILED",
        error,
      );
    }
  }

  /**
   * Estimate authorization needed for storing data
   */
  estimateAuthorization(dataSize: number): {
    transactions: number;
    bytes: number;
  } {
    const numChunks = Math.ceil(dataSize / this.config.defaultChunkSize);
    let transactions = numChunks;
    let bytes = dataSize;

    if (this.config.createManifest) {
      transactions += 1;
      bytes += numChunks * 10 + 1000;
    }

    return { transactions, bytes };
  }
}
