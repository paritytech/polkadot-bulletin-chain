// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, expect, it } from "vitest"
import { type ChunkerConfig, FixedSizeChunker } from "../../src/chunker"

describe("Chunker", () => {
  it("should chunk data correctly with default config", () => {
    const data = new Uint8Array(5 * 1024 * 1024).fill(0xaa) // 5 MiB
    const config: ChunkerConfig = {
      chunkSize: 1024 * 1024, // 1 MiB
      maxParallel: 8,
      createManifest: true,
    }

    const chunker = new FixedSizeChunker(config)
    const chunks = chunker.chunk(data)

    expect(chunks).toHaveLength(5)

    chunks.forEach((chunk, i) => {
      expect(chunk.data).toHaveLength(1024 * 1024)
      expect(chunk.index).toBe(i)
      expect(chunk.totalChunks).toBe(5)
    })
  })

  it("should handle data smaller than chunk size", () => {
    const data = new Uint8Array(512 * 1024).fill(0xbb) // 512 KiB
    const config: ChunkerConfig = {
      chunkSize: 1024 * 1024, // 1 MiB
      maxParallel: 8,
      createManifest: true,
    }

    const chunker = new FixedSizeChunker(config)
    const chunks = chunker.chunk(data)

    expect(chunks).toHaveLength(1)
    expect(chunks[0].data).toHaveLength(512 * 1024)
    expect(chunks[0].index).toBe(0)
    expect(chunks[0].totalChunks).toBe(1)
  })

  it("should handle data with partial last chunk", () => {
    const data = new Uint8Array(2.5 * 1024 * 1024).fill(0xcc) // 2.5 MiB
    const config: ChunkerConfig = {
      chunkSize: 1024 * 1024, // 1 MiB
      maxParallel: 8,
      createManifest: true,
    }

    const chunker = new FixedSizeChunker(config)
    const chunks = chunker.chunk(data)

    expect(chunks).toHaveLength(3)
    expect(chunks[0].data).toHaveLength(1024 * 1024)
    expect(chunks[1].data).toHaveLength(1024 * 1024)
    expect(chunks[2].data).toHaveLength(0.5 * 1024 * 1024) // Last chunk is 0.5 MiB
  })

  it("should calculate total chunks correctly", () => {
    const config: ChunkerConfig = {
      chunkSize: 1024 * 1024, // 1 MiB
      maxParallel: 8,
      createManifest: true,
    }

    const chunker = new FixedSizeChunker(config)

    expect(chunker.numChunks(1024 * 1024)).toBe(1)
    expect(chunker.numChunks(5 * 1024 * 1024)).toBe(5)
    expect(chunker.numChunks(2.5 * 1024 * 1024)).toBe(3)
    expect(chunker.numChunks(0)).toBe(0)
  })

  it("should throw error for chunk size exceeding maximum", () => {
    const config: ChunkerConfig = {
      chunkSize: 10 * 1024 * 1024, // 10 MiB > MAX (2 MiB)
      maxParallel: 8,
      createManifest: true,
    }

    expect(() => new FixedSizeChunker(config)).toThrow()
  })

  it("should calculate chunks correctly for 64 MiB file", () => {
    const config: ChunkerConfig = {
      chunkSize: 2 * 1024 * 1024, // 2 MiB (MAX_CHUNK_SIZE)
      maxParallel: 8,
      createManifest: true,
    }

    const chunker = new FixedSizeChunker(config)

    // 64 MiB / 2 MiB = 32 chunks
    expect(chunker.numChunks(64 * 1024 * 1024)).toBe(32)

    expect(chunker.chunkSize).toBe(2 * 1024 * 1024)
  })

  it("should throw error for zero chunk size", () => {
    const config: ChunkerConfig = {
      chunkSize: 0,
      maxParallel: 8,
      createManifest: true,
    }

    expect(() => new FixedSizeChunker(config)).toThrow()
  })

  it("should validate chunk data integrity", () => {
    const data = new Uint8Array(3 * 1024 * 1024)
    for (let i = 0; i < data.length; i++) {
      data[i] = i % 256
    }

    const config: ChunkerConfig = {
      chunkSize: 1024 * 1024,
      maxParallel: 8,
      createManifest: true,
    }

    const chunker = new FixedSizeChunker(config)
    const chunks = chunker.chunk(data)

    // Reassemble and verify
    const reassembled = new Uint8Array(data.length)
    let offset = 0
    for (const chunk of chunks) {
      reassembled.set(chunk.data, offset)
      offset += chunk.data.length
    }

    expect(reassembled).toEqual(data)
  })
})
