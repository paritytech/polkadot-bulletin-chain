// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, expect, it } from "vitest"
import { BulletinPreparer } from "../../src/preparer"
import { BulletinError, ErrorCode, type HashAlgorithm } from "../../src/types"
import type { TransactionStatusEvent } from "../../src/types"
import { calculateCid, cidFromBytes, parseCid } from "../../src/utils"

describe("Error Handling", () => {
  describe("BulletinError", () => {
    it("should create error with code", () => {
      const error = new BulletinError("Test error", "TEST_CODE")

      expect(error.message).toBe("Test error")
      expect(error.code).toBe("TEST_CODE")
      expect(error.name).toBe("BulletinError")
      expect(error.cause).toBeUndefined()
    })

    it("should create error with cause", () => {
      const cause = new Error("Original error")
      const error = new BulletinError("Wrapped error", "WRAPPED", cause)

      expect(error.message).toBe("Wrapped error")
      expect(error.code).toBe("WRAPPED")
      expect(error.cause).toBe(cause)
    })

    it("should be instanceof Error", () => {
      const error = new BulletinError("Test", "CODE")

      expect(error).toBeInstanceOf(Error)
      expect(error).toBeInstanceOf(BulletinError)
    })

    it("should preserve stack trace", () => {
      const error = new BulletinError("Test", "CODE")

      expect(error.stack).toBeDefined()
      expect(error.stack).toContain("BulletinError")
    })
  })

  describe("Async Error Propagation", () => {
    it("should propagate BulletinError through async chain", async () => {
      const asyncFunction = async () => {
        throw new BulletinError("Async error", "ASYNC_ERROR")
      }

      await expect(asyncFunction()).rejects.toThrow(BulletinError)
      await expect(asyncFunction()).rejects.toMatchObject({
        code: "ASYNC_ERROR",
        message: "Async error",
      })
    })

    it("should preserve error type through Promise.all", async () => {
      const promises = [
        Promise.resolve(1),
        Promise.reject(new BulletinError("Error in promise", "PROMISE_ERROR")),
        Promise.resolve(3),
      ]

      try {
        await Promise.all(promises)
        expect.fail("Should have thrown")
      } catch (error) {
        expect(error).toBeInstanceOf(BulletinError)
        expect((error as BulletinError).code).toBe("PROMISE_ERROR")
      }
    })

    it("should preserve error type through Promise.allSettled", async () => {
      const promises = [
        Promise.resolve(1),
        Promise.reject(new BulletinError("Error", "SETTLED_ERROR")),
        Promise.resolve(3),
      ]

      const results = await Promise.allSettled(promises)

      expect(results[0].status).toBe("fulfilled")
      expect(results[1].status).toBe("rejected")
      expect(results[2].status).toBe("fulfilled")

      if (results[1].status === "rejected") {
        expect(results[1].reason).toBeInstanceOf(BulletinError)
        expect((results[1].reason as BulletinError).code).toBe("SETTLED_ERROR")
      }
    })
  })

  describe("Client Error Handling", () => {
    it("should throw BulletinError for empty data in prepareStore", async () => {
      const preparer = new BulletinPreparer()

      await expect(preparer.prepareStore(new Uint8Array(0))).rejects.toThrow(
        BulletinError,
      )
      await expect(
        preparer.prepareStore(new Uint8Array(0)),
      ).rejects.toMatchObject({
        code: "EMPTY_DATA",
      })
    })

    it("should throw DATA_TOO_LARGE for data exceeding chunkingThreshold in prepareStore", async () => {
      const preparer = new BulletinPreparer({ chunkingThreshold: 1024 })
      const oversized = new Uint8Array(1025)

      await expect(preparer.prepareStore(oversized)).rejects.toThrow(
        BulletinError,
      )
      await expect(preparer.prepareStore(oversized)).rejects.toMatchObject({
        code: "DATA_TOO_LARGE",
      })
    })

    it("should throw BulletinError for empty data in prepareStoreChunked", async () => {
      const preparer = new BulletinPreparer()

      await expect(
        preparer.prepareStoreChunked(new Uint8Array(0)),
      ).rejects.toThrow(BulletinError)
      await expect(
        preparer.prepareStoreChunked(new Uint8Array(0)),
      ).rejects.toMatchObject({
        code: "EMPTY_DATA",
      })
    })
  })

  describe("CID Error Handling", () => {
    it("should throw BulletinError for invalid CID string", () => {
      expect(() => parseCid("not-a-valid-cid")).toThrow(BulletinError)
      expect(() => parseCid("not-a-valid-cid")).toThrow("Failed to parse CID")
    })

    it("should throw BulletinError for empty CID string", () => {
      expect(() => parseCid("")).toThrow(BulletinError)
    })

    it("should throw BulletinError for invalid CID bytes", () => {
      const invalidBytes = new Uint8Array([0xff, 0xff, 0xff])
      expect(() => cidFromBytes(invalidBytes)).toThrow(BulletinError)
    })

    it("should throw BulletinError for empty CID bytes", () => {
      expect(() => cidFromBytes(new Uint8Array(0))).toThrow(BulletinError)
    })

    it("should throw BulletinError for unsupported hash algorithm", async () => {
      const data = new Uint8Array([1, 2, 3, 4, 5])

      // Use an invalid hash algorithm code
      await expect(
        calculateCid(data, 0x55, 0xff as HashAlgorithm),
      ).rejects.toThrow(BulletinError)
    })
  })

  describe("Error Message Quality", () => {
    it("should include useful context in error messages", async () => {
      const preparer = new BulletinPreparer()

      try {
        await preparer.prepareStore(new Uint8Array(0))
        expect.fail("Should have thrown")
      } catch (error) {
        expect(error).toBeInstanceOf(BulletinError)
        const bulletinError = error as BulletinError

        // Error should have meaningful message
        expect(bulletinError.message.length).toBeGreaterThan(10)

        // Error should have a code
        expect(bulletinError.code).toBeDefined()
        expect(bulletinError.code.length).toBeGreaterThan(0)
      }
    })

    it("should include cause when wrapping errors", () => {
      const originalError = new TypeError("Cannot read property of undefined")
      const wrappedError = new BulletinError(
        "Operation failed",
        "OP_FAILED",
        originalError,
      )

      expect(wrappedError.cause).toBe(originalError)
      expect((wrappedError.cause as Error).message).toContain("undefined")
    })
  })

  describe("ErrorCode enum", () => {
    it("should have all expected codes as string values", () => {
      // ErrorCode values equal their key names (string enum)
      expect(ErrorCode.EMPTY_DATA).toBe("EMPTY_DATA")
      expect(ErrorCode.FILE_TOO_LARGE).toBe("FILE_TOO_LARGE")
      expect(ErrorCode.CHUNK_TOO_LARGE).toBe("CHUNK_TOO_LARGE")
      expect(ErrorCode.INVALID_CHUNK_SIZE).toBe("INVALID_CHUNK_SIZE")
      expect(ErrorCode.INVALID_CONFIG).toBe("INVALID_CONFIG")
      expect(ErrorCode.INVALID_CID).toBe("INVALID_CID")
      expect(ErrorCode.UNSUPPORTED_HASH_ALGORITHM).toBe("UNSUPPORTED_HASH_ALGORITHM")
      expect(ErrorCode.INVALID_HASH_ALGORITHM).toBe("INVALID_HASH_ALGORITHM")
      expect(ErrorCode.CID_CALCULATION_FAILED).toBe("CID_CALCULATION_FAILED")
      expect(ErrorCode.DAG_ENCODING_FAILED).toBe("DAG_ENCODING_FAILED")
      expect(ErrorCode.DAG_DECODING_FAILED).toBe("DAG_DECODING_FAILED")
      expect(ErrorCode.AUTHORIZATION_NOT_FOUND).toBe("AUTHORIZATION_NOT_FOUND")
      expect(ErrorCode.INSUFFICIENT_AUTHORIZATION).toBe("INSUFFICIENT_AUTHORIZATION")
      expect(ErrorCode.AUTHORIZATION_EXPIRED).toBe("AUTHORIZATION_EXPIRED")
      expect(ErrorCode.AUTHORIZATION_FAILED).toBe("AUTHORIZATION_FAILED")
      expect(ErrorCode.SUBMISSION_FAILED).toBe("SUBMISSION_FAILED")
      expect(ErrorCode.TRANSACTION_FAILED).toBe("TRANSACTION_FAILED")
      expect(ErrorCode.STORAGE_FAILED).toBe("STORAGE_FAILED")
      expect(ErrorCode.NETWORK_ERROR).toBe("NETWORK_ERROR")
      expect(ErrorCode.CHUNKING_FAILED).toBe("CHUNKING_FAILED")
      expect(ErrorCode.CHUNK_FAILED).toBe("CHUNK_FAILED")
      expect(ErrorCode.RETRIEVAL_FAILED).toBe("RETRIEVAL_FAILED")
      expect(ErrorCode.RENEWAL_NOT_FOUND).toBe("RENEWAL_NOT_FOUND")
      expect(ErrorCode.RENEWAL_FAILED).toBe("RENEWAL_FAILED")
      expect(ErrorCode.TIMEOUT).toBe("TIMEOUT")
      expect(ErrorCode.UNSUPPORTED_OPERATION).toBe("UNSUPPORTED_OPERATION")
      expect(ErrorCode.RETRY_EXHAUSTED).toBe("RETRY_EXHAUSTED")
    })

    it("should be usable with BulletinError", () => {
      const error = new BulletinError("test", ErrorCode.EMPTY_DATA)
      expect(error.code).toBe("EMPTY_DATA")
    })

    it("should remain backward compatible with string comparisons", () => {
      const error = new BulletinError("test", ErrorCode.EMPTY_DATA)
      expect(error.code === "EMPTY_DATA").toBe(true)
    })
  })

  describe("BulletinError retryable getter", () => {
    it("should return true for retryable error codes", () => {
      const retryableCodes = [
        ErrorCode.AUTHORIZATION_EXPIRED,
        ErrorCode.NETWORK_ERROR,
        ErrorCode.STORAGE_FAILED,
        ErrorCode.SUBMISSION_FAILED,
        ErrorCode.TRANSACTION_FAILED,
        ErrorCode.RETRIEVAL_FAILED,
        ErrorCode.RENEWAL_FAILED,
        ErrorCode.TIMEOUT,
      ]

      for (const code of retryableCodes) {
        const error = new BulletinError("test", code)
        expect(error.retryable).toBe(true)
      }
    })

    it("should return false for non-retryable error codes", () => {
      const nonRetryableCodes = [
        ErrorCode.EMPTY_DATA,
        ErrorCode.FILE_TOO_LARGE,
        ErrorCode.CHUNK_TOO_LARGE,
        ErrorCode.INVALID_CHUNK_SIZE,
        ErrorCode.INVALID_CONFIG,
        ErrorCode.INVALID_CID,
        ErrorCode.UNSUPPORTED_HASH_ALGORITHM,
        ErrorCode.CID_CALCULATION_FAILED,
        ErrorCode.DAG_ENCODING_FAILED,
        ErrorCode.DAG_DECODING_FAILED,
        ErrorCode.AUTHORIZATION_NOT_FOUND,
        ErrorCode.INSUFFICIENT_AUTHORIZATION,
        ErrorCode.AUTHORIZATION_FAILED,
        ErrorCode.CHUNKING_FAILED,
        ErrorCode.CHUNK_FAILED,
        ErrorCode.RENEWAL_NOT_FOUND,
        ErrorCode.UNSUPPORTED_OPERATION,
        ErrorCode.RETRY_EXHAUSTED,
      ]

      for (const code of nonRetryableCodes) {
        const error = new BulletinError("test", code)
        expect(error.retryable).toBe(false)
      }
    })

    it("should return false for unknown codes", () => {
      const error = new BulletinError("test", "UNKNOWN_CODE")
      expect(error.retryable).toBe(false)
    })
  })

  describe("BulletinError recoveryHint getter", () => {
    it("should return actionable hints for all ErrorCode values", () => {
      for (const code of Object.values(ErrorCode)) {
        const error = new BulletinError("test", code)
        expect(error.recoveryHint).toBeDefined()
        expect(error.recoveryHint.length).toBeGreaterThan(0)
        expect(error.recoveryHint).not.toBe("No recovery hint available")
      }
    })

    it("should return fallback for unknown codes", () => {
      const error = new BulletinError("test", "UNKNOWN_CODE")
      expect(error.recoveryHint).toBe("No recovery hint available")
    })
  })

  describe("TransactionStatusEvent variants", () => {
    it("should support validated event", () => {
      const event: TransactionStatusEvent = { type: "validated" }
      expect(event.type).toBe("validated")
    })

    it("should support broadcasted event with numPeers", () => {
      const event: TransactionStatusEvent = { type: "broadcasted", numPeers: 5 }
      expect(event.type).toBe("broadcasted")
      expect(event.numPeers).toBe(5)
    })

    it("should support in_best_block event", () => {
      const event: TransactionStatusEvent = {
        type: "in_best_block",
        blockHash: "0xabc",
        blockNumber: 42,
        txIndex: 1,
      }
      expect(event.type).toBe("in_best_block")
      expect(event.blockHash).toBe("0xabc")
      expect(event.blockNumber).toBe(42)
      expect(event.txIndex).toBe(1)
    })

    it("should support no_longer_in_best_block event", () => {
      const event: TransactionStatusEvent = { type: "no_longer_in_best_block" }
      expect(event.type).toBe("no_longer_in_best_block")
    })

    it("should support invalid event", () => {
      const event: TransactionStatusEvent = { type: "invalid", error: "nonce too low" }
      expect(event.type).toBe("invalid")
      expect(event.error).toBe("nonce too low")
    })

    it("should support dropped event", () => {
      const event: TransactionStatusEvent = { type: "dropped", error: "pool full" }
      expect(event.type).toBe("dropped")
      expect(event.error).toBe("pool full")
    })

    it("should still support deprecated best_block event", () => {
      const event: TransactionStatusEvent = {
        type: "best_block",
        blockHash: "0xabc",
        blockNumber: 42,
      }
      expect(event.type).toBe("best_block")
    })
  })
})
