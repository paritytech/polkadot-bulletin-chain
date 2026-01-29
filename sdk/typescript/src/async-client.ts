// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Async client with full transaction submission support
 */

import { CID } from 'multiformats/cid';
import { FixedSizeChunker, reassembleChunks } from './chunker.js';
import { UnixFsDagBuilder } from './dag.js';
import { calculateCid } from './utils.js';
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
  ChunkDetails,
} from './types.js';
import { TransactionSubmitter, TransactionReceipt } from './transaction.js';

/**
 * Async Bulletin client that submits transactions to the chain
 *
 * This client provides a complete interface for storing data on Bulletin Chain,
 * handling everything from chunking to transaction submission.
 */
export class AsyncBulletinClient {
  private submitter: TransactionSubmitter;
  private config: Required<Omit<ClientConfig, 'endpoint'>> & { chunkingThreshold: number; checkAuthorizationBeforeUpload: boolean };
  private account?: string;

  constructor(submitter: TransactionSubmitter, config?: Partial<ClientConfig>) {
    this.submitter = submitter;
    this.config = {
      defaultChunkSize: config?.defaultChunkSize ?? 1024 * 1024,
      maxParallel: config?.maxParallel ?? 8,
      createManifest: config?.createManifest ?? true,
      chunkingThreshold: config?.chunkingThreshold ?? 2 * 1024 * 1024, // 2 MiB
      checkAuthorizationBeforeUpload: config?.checkAuthorizationBeforeUpload ?? true,
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
   * Store data on Bulletin Chain
   *
   * Automatically chunks data if it exceeds the configured threshold.
   * This handles the complete workflow:
   * 1. Decide whether to chunk based on data size
   * 2. Calculate CID(s)
   * 3. Submit transaction(s)
   * 4. Wait for finalization
   *
   * @param data - Data to store
   * @param options - Storage options (CID codec, hash algorithm)
   * @param progressCallback - Optional callback for progress tracking (only called for chunked uploads)
   */
  async store(
    data: Uint8Array,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
  ): Promise<StoreResult> {
    if (data.length === 0) {
      throw new BulletinError('Data cannot be empty', 'EMPTY_DATA');
    }

    // Decide whether to chunk based on threshold
    if (data.length > this.config.chunkingThreshold) {
      // Large data - use chunking
      return this.storeInternalChunked(data, undefined, options, progressCallback);
    } else {
      // Small data - single transaction
      return this.storeInternalSingle(data, options);
    }
  }

  /**
   * Internal: Store data in a single transaction (no chunking)
   */
  private async storeInternalSingle(
    data: Uint8Array,
    options?: StoreOptions,
  ): Promise<StoreResult> {
    if (data.length === 0) {
      throw new BulletinError('Data cannot be empty', 'EMPTY_DATA');
    }

    // Check authorization before upload if enabled
    if (this.config.checkAuthorizationBeforeUpload && this.account) {
      if (this.submitter.queryAccountAuthorization) {
        // Query current authorization
        const auth = await this.submitter.queryAccountAuthorization(this.account);

        if (auth) {
          // Check if authorization has expired
          if (auth.expiresAt !== undefined) {
            if (this.submitter.queryCurrentBlock) {
              const currentBlock = await this.submitter.queryCurrentBlock();
              if (currentBlock !== undefined && auth.expiresAt <= currentBlock) {
                throw new BulletinError(
                  `Authorization expired at block ${auth.expiresAt} (current block: ${currentBlock})`,
                  'AUTHORIZATION_EXPIRED',
                  { expiredAt: auth.expiresAt, currentBlock },
                );
              }
            }
          }

          // Check if sufficient for this upload (1 transaction, data size)
          if (auth.maxSize < BigInt(data.length)) {
            throw new BulletinError(
              `Insufficient authorization: need ${data.length} bytes, have ${auth.maxSize} bytes`,
              'INSUFFICIENT_AUTHORIZATION',
              { need: data.length, available: auth.maxSize },
            );
          }

          if (auth.transactions < 1) {
            throw new BulletinError(
              `Insufficient authorization: need 1 transaction, have ${auth.transactions} transactions`,
              'INSUFFICIENT_AUTHORIZATION',
              { need: 1, available: auth.transactions },
            );
          }
        }
        // If no authorization found, let it proceed - on-chain validation will catch it
      }
    }

    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };

    // Calculate CID
    const cid = await calculateCid(
      data,
      opts.cidCodec ?? CidCodec.Raw,
      opts.hashingAlgorithm!,
    );

    // Submit transaction
    const receipt = await this.submitter.submitStore(data);

    return {
      cid,
      size: data.length,
      blockNumber: receipt.blockNumber,
      // No chunks for single upload
    };
  }

  /**
   * Calculate authorization requirements for chunked upload
   */
  private calculateRequirements(
    dataSize: number,
    numChunks: number,
    createManifest: boolean,
  ): { transactions: number; bytes: number } {
    // Each chunk needs one transaction
    let transactions = numChunks;

    // If creating manifest, add one more transaction
    if (createManifest) {
      transactions += 1;
    }

    // Total bytes = data size
    const bytes = dataSize;

    return { transactions, bytes };
  }

  /**
   * Internal: Store data with chunking (returns unified StoreResult)
   */
  private async storeInternalChunked(
    data: Uint8Array,
    config?: Partial<ChunkerConfig>,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
  ): Promise<StoreResult> {
    if (data.length === 0) {
      throw new BulletinError('Data cannot be empty', 'EMPTY_DATA');
    }

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

    // Check authorization before upload if enabled
    if (this.config.checkAuthorizationBeforeUpload && this.account) {
      if (this.submitter.queryAccountAuthorization) {
        // Calculate requirements
        const { transactions: txsNeeded, bytes: bytesNeeded } = this.calculateRequirements(
          data.length,
          chunks.length,
          chunkerConfig.createManifest,
        );

        // Query current authorization
        const auth = await this.submitter.queryAccountAuthorization(this.account);

        if (auth) {
          // Check if authorization has expired
          if (auth.expiresAt !== undefined) {
            if (this.submitter.queryCurrentBlock) {
              const currentBlock = await this.submitter.queryCurrentBlock();
              if (currentBlock !== undefined && auth.expiresAt <= currentBlock) {
                throw new BulletinError(
                  `Authorization expired at block ${auth.expiresAt} (current block: ${currentBlock})`,
                  'AUTHORIZATION_EXPIRED',
                  { expiredAt: auth.expiresAt, currentBlock },
                );
              }
            }
          }

          // Check if sufficient
          if (auth.maxSize < BigInt(bytesNeeded)) {
            throw new BulletinError(
              `Insufficient authorization: need ${bytesNeeded} bytes, have ${auth.maxSize} bytes`,
              'INSUFFICIENT_AUTHORIZATION',
              { need: bytesNeeded, available: auth.maxSize },
            );
          }

          if (auth.transactions < txsNeeded) {
            throw new BulletinError(
              `Insufficient authorization: need ${txsNeeded} transactions, have ${auth.transactions} transactions`,
              'INSUFFICIENT_AUTHORIZATION',
              { need: txsNeeded, available: auth.transactions },
            );
          }
        }
        // If no authorization found, let it proceed - on-chain validation will catch it
      }
    }

    const chunkCids: CID[] = [];
    let lastBlockNumber: number | undefined;

    // Submit each chunk
    for (const chunk of chunks) {
      if (progressCallback) {
        progressCallback({
          type: 'chunk_started',
          index: chunk.index,
          total: chunks.length,
        });
      }

      try {
        // Calculate CID for this chunk
        const cid = await calculateCid(
          chunk.data,
          opts.cidCodec ?? CidCodec.Raw,
          opts.hashingAlgorithm!,
        );

        chunk.cid = cid;

        // Submit chunk transaction
        const receipt = await this.submitter.submitStore(chunk.data);
        lastBlockNumber = receipt.blockNumber;

        chunkCids.push(cid);

        if (progressCallback) {
          progressCallback({
            type: 'chunk_completed',
            index: chunk.index,
            total: chunks.length,
            cid,
          });
        }
      } catch (error) {
        if (progressCallback) {
          progressCallback({
            type: 'chunk_failed',
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
        progressCallback({ type: 'manifest_started' });
      }

      const builder = new UnixFsDagBuilder();
      const manifest = await builder.build(chunks, opts.hashingAlgorithm!);

      // Submit manifest
      const receipt = await this.submitter.submitStore(manifest.dagBytes);
      lastBlockNumber = receipt.blockNumber;

      manifestCid = manifest.rootCid;

      if (progressCallback) {
        progressCallback({
          type: 'manifest_created',
          cid: manifest.rootCid,
        });
      }
    }

    if (progressCallback) {
      progressCallback({
        type: 'completed',
        manifestCid,
      });
    }

    // Return unified StoreResult
    return {
      cid: manifestCid ?? chunkCids[0]!,
      size: data.length,
      blockNumber: lastBlockNumber,
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
   */
  async storeChunked(
    data: Uint8Array,
    config?: Partial<ChunkerConfig>,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
  ): Promise<ChunkedStoreResult> {
    if (data.length === 0) {
      throw new BulletinError('Data cannot be empty', 'EMPTY_DATA');
    }

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

    // Submit each chunk
    for (const chunk of chunks) {
      if (progressCallback) {
        progressCallback({
          type: 'chunk_started',
          index: chunk.index,
          total: chunks.length,
        });
      }

      try {
        // Calculate CID for this chunk
        const cid = await calculateCid(
          chunk.data,
          opts.cidCodec ?? CidCodec.Raw,
          opts.hashingAlgorithm!,
        );

        chunk.cid = cid;

        // Submit chunk transaction
        await this.submitter.submitStore(chunk.data);

        chunkCids.push(cid);

        if (progressCallback) {
          progressCallback({
            type: 'chunk_completed',
            index: chunk.index,
            total: chunks.length,
            cid,
          });
        }
      } catch (error) {
        if (progressCallback) {
          progressCallback({
            type: 'chunk_failed',
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
        progressCallback({ type: 'manifest_started' });
      }

      const builder = new UnixFsDagBuilder();
      const manifest = await builder.build(chunks, opts.hashingAlgorithm!);

      // Submit manifest
      await this.submitter.submitStore(manifest.dagBytes);

      manifestCid = manifest.rootCid;

      if (progressCallback) {
        progressCallback({
          type: 'manifest_created',
          cid: manifest.rootCid,
        });
      }
    }

    if (progressCallback) {
      progressCallback({
        type: 'completed',
        manifestCid,
      });
    }

    return {
      chunkCids,
      manifestCid,
      totalSize: data.length,
      numChunks: chunks.length,
    };
  }

  /**
   * Authorize an account to store data
   *
   * Requires sudo/authorizer privileges
   */
  async authorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
  ): Promise<TransactionReceipt> {
    return this.submitter.submitAuthorizeAccount(who, transactions, bytes);
  }

  /**
   * Authorize a preimage (by content hash) to be stored
   *
   * Requires sudo/authorizer privileges
   */
  async authorizePreimage(
    contentHash: Uint8Array,
    maxSize: bigint,
  ): Promise<TransactionReceipt> {
    return this.submitter.submitAuthorizePreimage(contentHash, maxSize);
  }

  /**
   * Renew/extend retention period for stored data
   */
  async renew(block: number, index: number): Promise<TransactionReceipt> {
    return this.submitter.submitRenew(block, index);
  }

  /**
   * Refresh an account authorization (extends expiry)
   *
   * Requires sudo/authorizer privileges
   */
  async refreshAccountAuthorization(who: string): Promise<TransactionReceipt> {
    return this.submitter.submitRefreshAccountAuthorization(who);
  }

  /**
   * Refresh a preimage authorization (extends expiry)
   *
   * Requires sudo/authorizer privileges
   */
  async refreshPreimageAuthorization(contentHash: Uint8Array): Promise<TransactionReceipt> {
    return this.submitter.submitRefreshPreimageAuthorization(contentHash);
  }

  /**
   * Remove an expired account authorization
   */
  async removeExpiredAccountAuthorization(who: string): Promise<TransactionReceipt> {
    return this.submitter.submitRemoveExpiredAccountAuthorization(who);
  }

  /**
   * Remove an expired preimage authorization
   */
  async removeExpiredPreimageAuthorization(contentHash: Uint8Array): Promise<TransactionReceipt> {
    return this.submitter.submitRemoveExpiredPreimageAuthorization(contentHash);
  }

  /**
   * Estimate authorization needed for storing data
   */
  estimateAuthorization(dataSize: number): { transactions: number; bytes: number } {
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
