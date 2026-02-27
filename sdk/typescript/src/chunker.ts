// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Data chunking utilities for splitting large files into smaller pieces
 */

import {
  Chunk,
  ChunkerConfig,
  DEFAULT_CHUNKER_CONFIG,
  BulletinError,
} from "./types.js";

/** Maximum chunk size allowed (8 MiB, matches chain's MaxTransactionSize) */
export const MAX_CHUNK_SIZE = 8 * 1024 * 1024;

/** Maximum file size allowed (64 MiB) */
export const MAX_FILE_SIZE = 64 * 1024 * 1024;

/**
 * Fixed-size chunker that splits data into equal-sized chunks
 */
export class FixedSizeChunker {
  private config: ChunkerConfig;

  constructor(config?: Partial<ChunkerConfig>) {
    this.config = { ...DEFAULT_CHUNKER_CONFIG, ...config };

    // Validate configuration
    if (this.config.chunkSize <= 0) {
      throw new BulletinError(
        "Chunk size must be greater than 0",
        "INVALID_CONFIG",
      );
    }
    if (this.config.chunkSize > MAX_CHUNK_SIZE) {
      throw new BulletinError(
        `Chunk size ${this.config.chunkSize} exceeds maximum allowed size of ${MAX_CHUNK_SIZE}`,
        "CHUNK_TOO_LARGE",
      );
    }
  }

  /**
   * Split data into chunks
   */
  chunk(data: Uint8Array): Chunk[] {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA");
    }
    if (data.length > MAX_FILE_SIZE) {
      throw new BulletinError(
        `Data size ${data.length} exceeds maximum allowed size of ${MAX_FILE_SIZE} (64 MiB)`,
        "FILE_TOO_LARGE",
      );
    }

    const chunks: Chunk[] = [];
    const totalChunks = this.numChunks(data.length);

    for (let i = 0; i < totalChunks; i++) {
      const start = i * this.config.chunkSize;
      const end = Math.min(start + this.config.chunkSize, data.length);
      const chunkData = data.slice(start, end);

      chunks.push({
        data: chunkData,
        index: i,
        totalChunks,
      });
    }

    return chunks;
  }

  /**
   * Calculate the number of chunks needed for the given data size
   */
  numChunks(dataSize: number): number {
    if (dataSize === 0) return 0;
    return Math.ceil(dataSize / this.config.chunkSize);
  }

  /**
   * Get the chunk size
   */
  get chunkSize(): number {
    return this.config.chunkSize;
  }
}

/**
 * Reassemble chunks back into the original data
 */
export function reassembleChunks(chunks: Chunk[]): Uint8Array {
  if (chunks.length === 0) {
    throw new BulletinError("Cannot reassemble empty chunks", "EMPTY_DATA");
  }

  // Validate chunk indices are sequential
  for (let i = 0; i < chunks.length; i++) {
    if (chunks[i].index !== i) {
      throw new BulletinError(
        `Chunk index mismatch: expected ${i}, got ${chunks[i].index}`,
        "CHUNKING_FAILED",
      );
    }
  }

  // Calculate total size
  const totalSize = chunks.reduce((sum, chunk) => sum + chunk.data.length, 0);
  const result = new Uint8Array(totalSize);

  // Concatenate all chunks
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk.data, offset);
    offset += chunk.data.length;
  }

  return result;
}
