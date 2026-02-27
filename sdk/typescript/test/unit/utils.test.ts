// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, expect, it } from "vitest"
import {
  batch,
  bytesToHex,
  calculateThroughput,
  createProgressTracker,
  deepClone,
  estimateFees,
  formatBytes,
  formatThroughput,
  hexToBytes,
  isBrowser,
  isNode,
  isValidSS58,
  measureTime,
  optimalChunkSize,
  retry,
  sleep,
  truncate,
  validateChunkSize,
} from "../../src/utils"

describe("Utils", () => {
  describe("Hex Conversion", () => {
    it("should convert hex to bytes", () => {
      const bytes = hexToBytes("deadbeef")
      expect(bytes).toEqual(new Uint8Array([0xde, 0xad, 0xbe, 0xef]))
    })

    it("should handle hex with 0x prefix", () => {
      const bytes = hexToBytes("0xdeadbeef")
      expect(bytes).toEqual(new Uint8Array([0xde, 0xad, 0xbe, 0xef]))
    })

    it("should convert bytes to hex", () => {
      const hex = bytesToHex(new Uint8Array([0xde, 0xad, 0xbe, 0xef]))
      expect(hex).toBe("0xdeadbeef")
    })

    it("should roundtrip hex conversion", () => {
      const original = new Uint8Array([0x12, 0x34, 0x56, 0x78])
      const hex = bytesToHex(original)
      const decoded = hexToBytes(hex)
      expect(decoded).toEqual(original)
    })
  })

  describe("formatBytes", () => {
    it("should format bytes correctly", () => {
      expect(formatBytes(0)).toBe("0 Bytes")
      expect(formatBytes(1024)).toBe("1.00 KB")
      expect(formatBytes(1048576)).toBe("1.00 MB")
      expect(formatBytes(1073741824)).toBe("1.00 GB")
    })

    it("should respect decimal places", () => {
      expect(formatBytes(1536, 0)).toBe("2 KB")
      expect(formatBytes(1536, 1)).toBe("1.5 KB")
      expect(formatBytes(1536, 2)).toBe("1.50 KB")
    })
  })

  describe("validateChunkSize", () => {
    it("should validate valid chunk sizes", () => {
      expect(() => validateChunkSize(1024 * 1024)).not.toThrow()
      expect(() => validateChunkSize(2 * 1024 * 1024)).not.toThrow()
    })

    it("should reject zero size", () => {
      expect(() => validateChunkSize(0)).toThrow()
    })

    it("should reject negative size", () => {
      expect(() => validateChunkSize(-1)).toThrow()
    })

    it("should reject size exceeding maximum", () => {
      expect(() => validateChunkSize(10 * 1024 * 1024)).toThrow()
    })
  })

  describe("optimalChunkSize", () => {
    it("should return data size if smaller than minimum", () => {
      expect(optimalChunkSize(500_000)).toBe(500_000)
    })

    it("should return minimum for moderately sized data", () => {
      expect(optimalChunkSize(100_000_000)).toBe(1_048_576)
    })

    it("should return maximum for very large data", () => {
      expect(optimalChunkSize(1_000_000_000)).toBe(2_097_152)
    })
  })

  describe("estimateFees", () => {
    it("should estimate fees", () => {
      const fees = estimateFees(1_000_000)
      expect(fees).toBeGreaterThan(0n)
    })

    it("should increase with data size", () => {
      const fees1 = estimateFees(1_000_000)
      const fees2 = estimateFees(2_000_000)
      expect(fees2).toBeGreaterThan(fees1)
    })
  })

  describe("retry", () => {
    it("should retry on failure", async () => {
      let attempts = 0

      const result = await retry(
        async () => {
          attempts++
          if (attempts < 3) throw new Error("Fail")
          return "success"
        },
        { maxRetries: 3, delayMs: 10 },
      )

      expect(result).toBe("success")
      expect(attempts).toBe(3)
    })

    it("should throw after max retries", async () => {
      await expect(
        retry(
          async () => {
            throw new Error("Always fail")
          },
          { maxRetries: 2, delayMs: 10 },
        ),
      ).rejects.toThrow("Always fail")
    })

    it("should succeed on first try", async () => {
      let attempts = 0

      const result = await retry(
        async () => {
          attempts++
          return "success"
        },
        { maxRetries: 3, delayMs: 10 },
      )

      expect(result).toBe("success")
      expect(attempts).toBe(1)
    })
  })

  describe("sleep", () => {
    it("should wait specified time", async () => {
      const start = Date.now()
      await sleep(100)
      const duration = Date.now() - start

      expect(duration).toBeGreaterThanOrEqual(90) // Allow some tolerance
    })
  })

  describe("batch", () => {
    it("should batch array correctly", () => {
      const items = [1, 2, 3, 4, 5]
      const batches = batch(items, 2)

      expect(batches).toEqual([[1, 2], [3, 4], [5]])
    })

    it("should handle exact multiples", () => {
      const items = [1, 2, 3, 4]
      const batches = batch(items, 2)

      expect(batches).toEqual([
        [1, 2],
        [3, 4],
      ])
    })

    it("should handle empty array", () => {
      const batches = batch([], 2)
      expect(batches).toEqual([])
    })
  })

  describe("createProgressTracker", () => {
    it("should track progress", () => {
      const tracker = createProgressTracker(100)

      expect(tracker.current).toBe(0)
      expect(tracker.percentage).toBe(0)

      tracker.increment(25)
      expect(tracker.current).toBe(25)
      expect(tracker.percentage).toBe(25)

      tracker.increment(25)
      expect(tracker.current).toBe(50)
      expect(tracker.percentage).toBe(50)
    })

    it("should not exceed total", () => {
      const tracker = createProgressTracker(100)

      tracker.increment(150)
      expect(tracker.current).toBe(100)
      expect(tracker.percentage).toBe(100)
      expect(tracker.isComplete()).toBe(true)
    })

    it("should reset", () => {
      const tracker = createProgressTracker(100)

      tracker.increment(50)
      tracker.reset()

      expect(tracker.current).toBe(0)
      expect(tracker.percentage).toBe(0)
    })
  })

  describe("measureTime", () => {
    it("should measure execution time", async () => {
      const [result, duration] = await measureTime(async () => {
        await sleep(100)
        return "done"
      })

      expect(result).toBe("done")
      expect(duration).toBeGreaterThanOrEqual(90)
    })
  })

  describe("calculateThroughput", () => {
    it("should calculate bytes per second", () => {
      const throughput = calculateThroughput(1_048_576, 1000)
      expect(throughput).toBe(1_048_576)
    })

    it("should handle zero time", () => {
      const throughput = calculateThroughput(1000, 0)
      expect(throughput).toBe(0)
    })
  })

  describe("formatThroughput", () => {
    it("should format throughput", () => {
      const formatted = formatThroughput(1_048_576)
      expect(formatted).toContain("MB/s")
    })
  })

  describe("isValidSS58", () => {
    it("should validate valid SS58 addresses", () => {
      expect(
        isValidSS58("5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"),
      ).toBe(true)
      expect(
        isValidSS58("5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"),
      ).toBe(true)
    })

    it("should reject invalid addresses", () => {
      expect(isValidSS58("invalid")).toBe(false)
      expect(isValidSS58("0x1234")).toBe(false)
      expect(isValidSS58("")).toBe(false)
    })
  })

  describe("truncate", () => {
    it("should truncate long strings", () => {
      const truncated = truncate("bafkreiabcd1234567890", 15)
      expect(truncated).toBe("bafkre...567890")
      expect(truncated.length).toBe(15)
    })

    it("should not truncate short strings", () => {
      const notTruncated = truncate("short", 10)
      expect(notTruncated).toBe("short")
    })

    it("should use custom ellipsis", () => {
      const truncated = truncate("longstring", 8, "--")
      expect(truncated).toContain("--")
    })
  })

  describe("deepClone", () => {
    it("should deep clone objects", () => {
      const original = { a: 1, b: { c: 2 } }
      const cloned = deepClone(original)

      expect(cloned).toEqual(original)
      expect(cloned).not.toBe(original)
      expect(cloned.b).not.toBe(original.b)
    })

    it("should clone arrays", () => {
      const original = [1, 2, [3, 4]]
      const cloned = deepClone(original)

      expect(cloned).toEqual(original)
      expect(cloned).not.toBe(original)
    })
  })

  describe("Environment Detection", () => {
    it("should detect Node.js", () => {
      expect(isNode()).toBe(true)
    })

    it("should detect browser", () => {
      expect(isBrowser()).toBe(false)
    })
  })
})
