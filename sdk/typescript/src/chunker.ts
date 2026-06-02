// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Data chunking utilities for splitting large files into smaller pieces
 */

import type { BlobSource } from "./blob-source.js"
import {
  BulletinError,
  type Chunk,
  type ChunkerConfig,
  DEFAULT_CHUNKER_CONFIG,
  ErrorCode,
} from "./types.js"

/** Maximum chunk size allowed (2 MiB, Bitswap compatibility limit) */
export const MAX_CHUNK_SIZE = 2 * 1024 * 1024

/** Maximum file size the SDK will chunk in a single operation (64 MiB).
 * For larger files, split into segments of at most 64 MiB and chunk each independently. */
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
        ErrorCode.INVALID_CHUNK_SIZE,
      )
    }
    if (this.config.chunkSize > MAX_CHUNK_SIZE) {
      throw new BulletinError(
        `Chunk size ${this.config.chunkSize} exceeds maximum allowed size of ${MAX_CHUNK_SIZE}`,
        ErrorCode.CHUNK_TOO_LARGE,
      )
    }
  }

  /**
   * Split data into chunks
   */
  chunk(data: Uint8Array): Chunk[] {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", ErrorCode.EMPTY_DATA)
    }
    if (data.length > MAX_FILE_SIZE) {
      throw new BulletinError(
        `Data size ${data.length} exceeds maximum allowed size of ${MAX_FILE_SIZE} (64 MiB)`,
        ErrorCode.DATA_TOO_LARGE,
      )
    }

    const chunks: Chunk[] = []
    const totalChunks = this.numChunks(data.length)

    for (let i = 0; i < totalChunks; i++) {
      const start = i * this.config.chunkSize
      const end = Math.min(start + this.config.chunkSize, data.length)
      const chunkData = data.subarray(start, end)

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

/**
 * Re-slice an arbitrary-boundary byte stream into fixed `chunkSize` pieces.
 *
 * Carries a small remainder across input reads, so peak working memory is
 * ~`chunkSize` regardless of file size. The final chunk may be shorter. Used by
 * the streaming estimate/plan path; the in-memory {@link FixedSizeChunker} is
 * the eager, whole-buffer alternative.
 */
export async function* chunkStream(
  source: BlobSource,
  chunkSize: number,
): AsyncIterable<{ index: number; data: Uint8Array }> {
  if (chunkSize <= 0) {
    throw new BulletinError(
      "Chunk size must be greater than 0",
      ErrorCode.INVALID_CHUNK_SIZE,
    )
  }
  if (chunkSize > MAX_CHUNK_SIZE) {
    throw new BulletinError(
      `Chunk size ${chunkSize} exceeds maximum allowed size of ${MAX_CHUNK_SIZE}`,
      ErrorCode.CHUNK_TOO_LARGE,
    )
  }

  let buf: Uint8Array = new Uint8Array(0)
  let index = 0
  for await (const part of source.open()) {
    if (part.length === 0) continue
    if (buf.length === 0) {
      buf = part
    } else {
      const merged = new Uint8Array(buf.length + part.length)
      merged.set(buf, 0)
      merged.set(part, buf.length)
      buf = merged
    }
    while (buf.length >= chunkSize) {
      yield { index: index++, data: buf.subarray(0, chunkSize) }
      buf = buf.subarray(chunkSize)
    }
  }
  if (buf.length > 0) {
    yield { index: index++, data: buf }
  }
}

/**
 * Reassemble chunks back into the original data
 *
 * Chunks are sorted by index before concatenation to handle out-of-order input.
 *
 * @param chunks - Array of chunks to reassemble
 * @returns The original data as a single Uint8Array
 */
export function reassembleChunks(chunks: Chunk[]): Uint8Array {
  if (chunks.length === 0) {
    throw new BulletinError("No chunks to reassemble", ErrorCode.EMPTY_DATA)
  }

  // Sort by index to ensure correct order
  const sorted = [...chunks].sort((a, b) => a.index - b.index)

  // Validate indices are contiguous starting from 0
  for (let i = 0; i < sorted.length; i++) {
    if (sorted[i]?.index !== i) {
      throw new BulletinError(
        `Missing chunk at index ${i}`,
        ErrorCode.MISSING_CHUNK,
      )
    }
  }

  // Calculate total size and concatenate
  const totalSize = sorted.reduce((sum, chunk) => sum + chunk.data.length, 0)
  const result = new Uint8Array(totalSize)
  let offset = 0
  for (const chunk of sorted) {
    result.set(chunk.data, offset)
    offset += chunk.data.length
  }

  return result
}
