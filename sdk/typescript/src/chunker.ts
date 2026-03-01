// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Data chunking utilities for splitting large files into smaller pieces
 */

import {
  BulletinError,
  type Chunk,
  type ChunkerConfig,
  DEFAULT_CHUNKER_CONFIG,
} from "./types.js"

/** Maximum chunk size allowed (2 MiB, Bitswap compatibility limit) */
export const MAX_CHUNK_SIZE = 2 * 1024 * 1024

/** Maximum file size allowed (64 MiB) */
export const MAX_FILE_SIZE = 64 * 1024 * 1024

/**
 * Fixed-size chunker that splits data into equal-sized chunks
 */
export class FixedSizeChunker {
  private config: ChunkerConfig

  constructor(config?: Partial<ChunkerConfig>) {
    this.config = { ...DEFAULT_CHUNKER_CONFIG, ...config }

    // Validate configuration
    if (this.config.chunkSize <= 0) {
      throw new BulletinError(
        "Chunk size must be greater than 0",
        "INVALID_CONFIG",
      )
    }
    if (this.config.chunkSize > MAX_CHUNK_SIZE) {
      throw new BulletinError(
        `Chunk size ${this.config.chunkSize} exceeds maximum allowed size of ${MAX_CHUNK_SIZE}`,
        "CHUNK_TOO_LARGE",
      )
    }
  }

  /**
   * Split data into chunks
   */
  chunk(data: Uint8Array): Chunk[] {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA")
    }
    if (data.length > MAX_FILE_SIZE) {
      throw new BulletinError(
        `Data size ${data.length} exceeds maximum allowed size of ${MAX_FILE_SIZE} (64 MiB)`,
        "FILE_TOO_LARGE",
      )
    }

    const chunks: Chunk[] = []
    const totalChunks = this.numChunks(data.length)

    for (let i = 0; i < totalChunks; i++) {
      const start = i * this.config.chunkSize
      const end = Math.min(start + this.config.chunkSize, data.length)
      const chunkData = data.slice(start, end)

      chunks.push({
        data: chunkData,
        index: i,
        totalChunks,
      })
    }

    return chunks
  }

  /**
   * Calculate the number of chunks needed for the given data size
   */
  numChunks(dataSize: number): number {
    if (dataSize === 0) return 0
    return Math.ceil(dataSize / this.config.chunkSize)
  }

  /**
   * Get the chunk size
   */
  get chunkSize(): number {
    return this.config.chunkSize
  }
}
