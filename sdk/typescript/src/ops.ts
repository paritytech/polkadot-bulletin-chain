// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * High-level client for interacting with Bulletin Chain
 */

import type { CID } from "multiformats/cid"
import { FixedSizeChunker } from "./chunker.js"
import { UnixFsDagBuilder } from "./dag.js"
import {
  BulletinError,
  type Chunk,
  type ChunkerConfig,
  CidCodec,
  type ClientConfig,
  DEFAULT_CHUNKER_CONFIG,
  DEFAULT_STORE_OPTIONS,
  type ProgressCallback,
  type StoreOptions,
} from "./types.js"
import { calculateCid, estimateAuthorization } from "./utils.js"

/**
 * High-level client for Bulletin Chain operations
 *
 * This provides a simplified API for common operations like storing
 * and retrieving data, with automatic chunking and manifest creation.
 *
 * For full blockchain integration, use PAPI (@polkadot-api) to submit
 * transactions to the TransactionStorage pallet.
 */
export class BulletinOps {
  private config: Required<ClientConfig>

  constructor(config?: ClientConfig) {
    this.config = {
      defaultChunkSize: config?.defaultChunkSize ?? 1024 * 1024,
      createManifest: config?.createManifest ?? true,
      chunkingThreshold: config?.chunkingThreshold ?? 2 * 1024 * 1024,
    }
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
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA")
    }

    const opts = { ...DEFAULT_STORE_OPTIONS, ...options }

    const cidCodec = opts.cidCodec ?? CidCodec.Raw
    const hashAlgorithm =
      opts.hashingAlgorithm ?? DEFAULT_STORE_OPTIONS.hashingAlgorithm

    const cid = await calculateCid(data, cidCodec, hashAlgorithm)

    return { data, cid }
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
    chunks: Chunk[]
    manifest?: { data: Uint8Array; cid: CID }
  }> {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA")
    }

    const chunkerConfig: ChunkerConfig = {
      ...DEFAULT_CHUNKER_CONFIG,
      chunkSize: config?.chunkSize ?? this.config.defaultChunkSize,
      createManifest: config?.createManifest ?? this.config.createManifest,
    }

    const opts = { ...DEFAULT_STORE_OPTIONS, ...options }

    const cidCodec = opts.cidCodec ?? CidCodec.Raw
    const hashAlgorithm =
      opts.hashingAlgorithm ?? DEFAULT_STORE_OPTIONS.hashingAlgorithm

    // Chunk the data
    const chunker = new FixedSizeChunker(chunkerConfig)
    const chunks = chunker.chunk(data)

    // Calculate CIDs for each chunk
    for (const chunk of chunks) {
      if (progressCallback) {
        progressCallback({
          type: "chunk_started",
          index: chunk.index,
          total: chunks.length,
        })
      }

      try {
        chunk.cid = await calculateCid(chunk.data, cidCodec, hashAlgorithm)

        if (progressCallback) {
          progressCallback({
            type: "chunk_completed",
            index: chunk.index,
            total: chunks.length,
            cid: chunk.cid,
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

    // Optionally create manifest
    let manifest: { data: Uint8Array; cid: CID } | undefined
    if (chunkerConfig.createManifest) {
      if (progressCallback) {
        progressCallback({ type: "manifest_started" })
      }

      const builder = new UnixFsDagBuilder()
      const dagManifest = await builder.build(chunks, hashAlgorithm)

      manifest = {
        data: dagManifest.dagBytes,
        cid: dagManifest.rootCid,
      }

      if (progressCallback) {
        progressCallback({
          type: "manifest_created",
          cid: dagManifest.rootCid,
        })
      }
    }

    if (progressCallback) {
      progressCallback({
        type: "completed",
        manifestCid: manifest?.cid,
      })
    }

    return { chunks, manifest }
  }

  /**
   * Estimate authorization needed for storing data
   *
   * Returns (num_transactions, total_bytes) needed for authorization
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
