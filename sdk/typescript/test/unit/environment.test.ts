// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Environment compatibility tests
 *
 * Verifies that core SDK functionality works in both Node.js and
 * browser (simulated via happy-dom) environments.
 */

import { describe, expect, it } from "vitest"
import { FixedSizeChunker, reassembleChunks } from "../../src/chunker"
import { BulletinError } from "../../src/types"
import {
  bytesToHex,
  formatBytes,
  hexToBytes,
  isBrowser,
  isNode,
  optimalChunkSize,
  validateChunkSize,
} from "../../src/utils"

/**
 * Core SDK functionality that must work in Node.js environment.
 * The default vitest environment is "node" (configured in vitest.config.ts).
 */
describe("Node.js environment", () => {
  describe("environment detection", () => {
    it("should detect Node.js", () => {
      expect(isNode()).toBe(true)
    })

    it("should not detect browser", () => {
      expect(isBrowser()).toBe(false)
    })
  })

  describe("core functionality", () => {
    it("should chunk and reassemble data", () => {
      const data = new Uint8Array(3000)
      crypto.getRandomValues(data)

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
      expect(error.name).toBe("BulletinError")
    })
  })
})
