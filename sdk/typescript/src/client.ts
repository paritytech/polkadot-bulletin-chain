// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * High-level client for interacting with Bulletin Chain
 */

import { CID } from "multiformats/cid";
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
  ProgressCallback,
  ProgressEvent,
  BulletinError,
  Chunk,
  CidCodec,
} from "./types.js";

/**
 * High-level client for Bulletin Chain operations
 *
 * This provides a simplified API for common operations like storing
 * and retrieving data, with automatic chunking and manifest creation.
 *
 * For full blockchain integration, use PAPI (@polkadot-api) to submit
 * transactions to the TransactionStorage pallet.
 */
export class BulletinClient {
  private config: Required<ClientConfig>;

  constructor(config: ClientConfig) {
    this.config = {
      endpoint: config.endpoint,
      defaultChunkSize: config.defaultChunkSize ?? 1024 * 1024,
      maxParallel: config.maxParallel ?? 8,
      createManifest: config.createManifest ?? true,
      chunkingThreshold: config.chunkingThreshold ?? 2 * 1024 * 1024,
      checkAuthorizationBeforeUpload:
        config.checkAuthorizationBeforeUpload ?? true,
    };
  }

  /**
   * Prepare a simple store operation (data < 2 MiB)
   *
   * Returns the data and its CID. Use PAPI to submit to TransactionStorage.store
   */
  async prepareStore(
    data: Uint8Array,
    options?: StoreOptions,
  ): Promise<{ data: Uint8Array; cid: CID }> {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }

    const opts = { ...DEFAULT_STORE_OPTIONS, ...options };

    // Calculate CID using defaults if not specified
    const cidCodec = opts.cidCodec ?? CidCodec.Raw;
    const hashAlgorithm = opts.hashingAlgorithm ?? DEFAULT_STORE_OPTIONS.hashingAlgorithm;

    const cid = await calculateCid(data, cidCodec, hashAlgorithm);

    return { data, cid };
  }

  /**
   * Prepare a chunked store operation for large files
   *
   * This chunks the data, calculates CIDs, and optionally creates a DAG-PB manifest.
   * Returns chunk data and manifest that can be submitted via PAPI.
   */
  async prepareStoreChunked(
    data: Uint8Array,
    config?: Partial<ChunkerConfig>,
    options?: StoreOptions,
    progressCallback?: ProgressCallback,
  ): Promise<{
    chunks: Chunk[];
    manifest?: { data: Uint8Array; cid: CID };
  }> {
    if (data.length === 0) {
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
    const chunks = chunker.chunk(data);

    // Calculate CIDs for each chunk
    for (const chunk of chunks) {
      if (progressCallback) {
        progressCallback({
          type: "chunk_started",
          index: chunk.index,
          total: chunks.length,
        });
      }

      try {
        chunk.cid = await calculateCid(chunk.data, cidCodec, hashAlgorithm);

        if (progressCallback) {
          progressCallback({
            type: "chunk_completed",
            index: chunk.index,
            total: chunks.length,
            cid: chunk.cid,
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

    // Optionally create manifest
    let manifest: { data: Uint8Array; cid: CID } | undefined;
    if (chunkerConfig.createManifest) {
      if (progressCallback) {
        progressCallback({ type: "manifest_started" });
      }

      const builder = new UnixFsDagBuilder();
      const dagManifest = await builder.build(chunks, hashAlgorithm);

      manifest = {
        data: dagManifest.dagBytes,
        cid: dagManifest.rootCid,
      };

      if (progressCallback) {
        progressCallback({
          type: "manifest_created",
          cid: dagManifest.rootCid,
        });
      }
    }

    if (progressCallback) {
      progressCallback({
        type: "completed",
        manifestCid: manifest?.cid,
      });
    }

    return { chunks, manifest };
  }

  /**
   * Estimate authorization needed for storing data
   *
   * Returns (num_transactions, total_bytes) needed for authorization
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
      // Estimate manifest size (~10 bytes per chunk + 1KB overhead)
      bytes += numChunks * 10 + 1000;
    }

    return { transactions, bytes };
  }
}

/**
 * Example integration with PAPI (placeholder)
 *
 * ```typescript
 * import { createClient } from 'polkadot-api';
 * import { getWsProvider } from 'polkadot-api/ws-provider/web';
 *
 * // Connect to chain
 * const wsProvider = getWsProvider('ws://localhost:9944');
 * const papiClient = createClient(wsProvider);
 *
 * // Use Bulletin SDK
 * const bulletinClient = new BulletinClient({ endpoint: 'ws://localhost:9944' });
 * const { data, cid } = await bulletinClient.prepareStore(myData);
 *
 * // Submit via PAPI
 * // const tx = api.tx.TransactionStorage.store({ data });
 * // await tx.signAndSubmit(signer);
 * ```
 */
