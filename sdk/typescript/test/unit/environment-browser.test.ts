// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// @vitest-environment happy-dom

/**
 * Browser environment compatibility tests
 *
 * These tests run under happy-dom which simulates browser globals
 * (window, document, navigator, etc.) to verify the SDK works
 * when loaded in a browser context.
 */

import { describe, expect, it } from "vitest"
import { FixedSizeChunker, reassembleChunks } from "../../src/chunker"
import { BulletinError } from "../../src/types"
import {
  bytesToHex,
  formatBytes,
  hexToBytes,
  isBrowser,
  optimalChunkSize,
  validateChunkSize,
} from "../../src/utils"

describe("Browser environment (happy-dom)", () => {
  describe("environment detection", () => {
    it("should detect browser", () => {
      expect(isBrowser()).toBe(true)
    })

    it("should have window and document globals", () => {
      expect(typeof globalThis).toBe("object")
      expect("document" in globalThis).toBe(true)
    })
  })

  describe("core functionality", () => {
    it("should chunk and reassemble data", () => {
      const data = new Uint8Array(3000)
      for (let i = 0; i < data.length; i++) data[i] = i % 256

      const chunker = new FixedSizeChunker({ chunkSize: 1024 })
      const chunks = chunker.chunk(data)

      expect(chunks.length).toBe(3)

      const reassembled = reassembleChunks(chunks)
      expect(reassembled).toEqual(data)
    })

    it("should convert hex roundtrip", () => {
      const original = new Uint8Array([0xde, 0xad, 0xbe, 0xef])
      const hex = bytesToHex(original)
      const decoded = hexToBytes(hex)
      expect(decoded).toEqual(original)
    })

    it("should format bytes", () => {
      expect(formatBytes(1048576)).toBe("1.00 MB")
    })

    it("should validate chunk sizes", () => {
      expect(() => validateChunkSize(1024 * 1024)).not.toThrow()
      expect(() => validateChunkSize(10 * 1024 * 1024)).toThrow()
    })

    it("should calculate optimal chunk size", () => {
      expect(optimalChunkSize(500_000)).toBe(500_000)
    })

    it("should use BulletinError with cause chain", () => {
      const cause = new Error("root cause")
      const error = new BulletinError("wrapper", "TEST", cause)

      expect(error.message).toBe("wrapper")
      expect(error.code).toBe("TEST")
      expect(error.cause).toBe(cause)
    })
  })
})
