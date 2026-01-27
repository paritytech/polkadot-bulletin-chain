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
  private config: Required<Omit<ClientConfig, 'endpoint'>>;

  constructor(submitter: TransactionSubmitter, config?: Partial<ClientConfig>) {
    this.submitter = submitter;
    this.config = {
      defaultChunkSize: config?.defaultChunkSize ?? 1024 * 1024,
      maxParallel: config?.maxParallel ?? 8,
      createManifest: config?.createManifest ?? true,
    };
  }

  /**
   * Store data on Bulletin Chain (simple, < 8 MiB)
   *
   * Handles the complete workflow:
   * 1. Calculate CID
   * 2. Submit transaction
   * 3. Wait for finalization
   */
  async store(data: Uint8Array, options?: StoreOptions): Promise<StoreResult> {
    if (data.length === 0) {
      throw new BulletinError('Data cannot be empty', 'EMPTY_DATA');
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
