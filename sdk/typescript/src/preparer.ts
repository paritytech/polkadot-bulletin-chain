// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Offline data preparation for Bulletin Chain (CID calculation, chunking, DAG building)
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
  type StoreOptions,
} from "./types.js"
import { calculateCid, estimateAuthorization } from "./utils.js"

/**
 * Offline data preparer for Bulletin Chain
 *
 * Handles CID calculation, chunking, DAG-PB manifest creation, and
 * authorization estimation without any chain interaction.
 * Used internally by AsyncBulletinClient and MockBulletinClient.
 */
export class BulletinPreparer {
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
      try {
        chunk.cid = await calculateCid(chunk.data, cidCodec, hashAlgorithm)
      } catch (error) {
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
      const builder = new UnixFsDagBuilder()
      const dagManifest = await builder.build(chunks, hashAlgorithm)

      manifest = {
        data: dagManifest.dagBytes,
        cid: dagManifest.rootCid,
      }
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
