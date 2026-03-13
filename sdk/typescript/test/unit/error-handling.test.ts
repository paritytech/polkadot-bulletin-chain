// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, expect, it } from "vitest"
import { BulletinPreparer } from "../../src/preparer"
import {
  BulletinError,
  ErrorCode,
  type HashAlgorithm,
  type TransactionStatusEvent,
  TxStatus,
} from "../../src/types"
import { calculateCid, cidFromBytes, parseCid } from "../../src/utils"

describe("Error Handling", () => {
  describe("BulletinError", () => {
    it("should create error with code", () => {
      const error = new BulletinError("Test error", ErrorCode.EMPTY_DATA)

      expect(error.message).toBe("Test error")
      expect(error.code).toBe(ErrorCode.EMPTY_DATA)
      expect(error.name).toBe("BulletinError")
      expect(error.cause).toBeUndefined()
    })

    it("should create error with cause", () => {
      const cause = new Error("Original error")
      const error = new BulletinError(
        "Wrapped error",
        ErrorCode.TRANSACTION_FAILED,
        cause,
      )

      expect(error.message).toBe("Wrapped error")
      expect(error.code).toBe(ErrorCode.TRANSACTION_FAILED)
      expect(error.cause).toBe(cause)
    })

    it("should be instanceof Error", () => {
      const error = new BulletinError("Test", ErrorCode.INVALID_CID)

      expect(error).toBeInstanceOf(Error)
      expect(error).toBeInstanceOf(BulletinError)
    })

    it("should preserve stack trace", () => {
      const error = new BulletinError("Test", ErrorCode.INVALID_CID)

      expect(error.stack).toBeDefined()
      expect(error.stack).toContain("BulletinError")
    })
  })

  describe("Async Error Propagation", () => {
    it("should propagate BulletinError through async chain", async () => {
      const asyncFunction = async () => {
        throw new BulletinError("Async error", ErrorCode.TRANSACTION_FAILED)
      }

      await expect(asyncFunction()).rejects.toThrow(BulletinError)
      await expect(asyncFunction()).rejects.toMatchObject({
        code: ErrorCode.TRANSACTION_FAILED,
        message: "Async error",
      })
    })

    it("should preserve error type through Promise.all", async () => {
      const promises = [
        Promise.resolve(1),
        Promise.reject(
          new BulletinError("Error in promise", ErrorCode.TRANSACTION_FAILED),
        ),
        Promise.resolve(3),
      ]

      try {
        await Promise.all(promises)
        expect.fail("Should have thrown")
      } catch (error) {
        expect(error).toBeInstanceOf(BulletinError)
        expect((error as BulletinError).code).toBe(ErrorCode.TRANSACTION_FAILED)
      }
    })

    it("should preserve error type through Promise.allSettled", async () => {
      const promises = [
        Promise.resolve(1),
        Promise.reject(new BulletinError("Error", ErrorCode.CHUNK_FAILED)),
        Promise.resolve(3),
      ]

      const results = await Promise.allSettled(promises)

      expect(results[0].status).toBe("fulfilled")
      expect(results[1].status).toBe("rejected")
      expect(results[2].status).toBe("fulfilled")

      if (results[1].status === "rejected") {
        expect(results[1].reason).toBeInstanceOf(BulletinError)
        expect((results[1].reason as BulletinError).code).toBe(
          ErrorCode.CHUNK_FAILED,
        )
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
        code: ErrorCode.EMPTY_DATA,
      })
    })

    it("should throw DATA_TOO_LARGE for data exceeding chunkingThreshold in prepareStore", async () => {
      const preparer = new BulletinPreparer({ chunkingThreshold: 1024 })
      const oversized = new Uint8Array(1025)

      await expect(preparer.prepareStore(oversized)).rejects.toThrow(
        BulletinError,
      )
      await expect(preparer.prepareStore(oversized)).rejects.toMatchObject({
        code: ErrorCode.DATA_TOO_LARGE,
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
        code: ErrorCode.EMPTY_DATA,
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
        ErrorCode.CHUNK_FAILED,
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
      expect(ErrorCode.DATA_TOO_LARGE).toBe("DATA_TOO_LARGE")
      expect(ErrorCode.CHUNK_TOO_LARGE).toBe("CHUNK_TOO_LARGE")
      expect(ErrorCode.INVALID_CHUNK_SIZE).toBe("INVALID_CHUNK_SIZE")
      expect(ErrorCode.INVALID_CONFIG).toBe("INVALID_CONFIG")
      expect(ErrorCode.INVALID_CID).toBe("INVALID_CID")
      expect(ErrorCode.INVALID_HASH_ALGORITHM).toBe("INVALID_HASH_ALGORITHM")
      expect(ErrorCode.CID_CALCULATION_FAILED).toBe("CID_CALCULATION_FAILED")
      expect(ErrorCode.DAG_ENCODING_FAILED).toBe("DAG_ENCODING_FAILED")
      expect(ErrorCode.INSUFFICIENT_AUTHORIZATION).toBe(
        "INSUFFICIENT_AUTHORIZATION",
      )
      expect(ErrorCode.AUTHORIZATION_FAILED).toBe("AUTHORIZATION_FAILED")
      expect(ErrorCode.TRANSACTION_FAILED).toBe("TRANSACTION_FAILED")
      expect(ErrorCode.CHUNK_FAILED).toBe("CHUNK_FAILED")
      expect(ErrorCode.MISSING_CHUNK).toBe("MISSING_CHUNK")
      expect(ErrorCode.TIMEOUT).toBe("TIMEOUT")
      expect(ErrorCode.UNSUPPORTED_OPERATION).toBe("UNSUPPORTED_OPERATION")
    })

    it("should be usable with BulletinError", () => {
      const error = new BulletinError("test", ErrorCode.EMPTY_DATA)
      expect(error.code).toBe("EMPTY_DATA")
    })

    it("should remain backward compatible with string comparisons", () => {
      const error = new BulletinError("test", ErrorCode.EMPTY_DATA)
      // biome-ignore lint/suspicious/noExplicitAny: testing backward compat with string comparison
      expect(error.code === ("EMPTY_DATA" as any)).toBe(true)
    })
  })

  describe("BulletinError retryable getter", () => {
    it("should return true for retryable error codes", () => {
      const retryableCodes = [ErrorCode.TRANSACTION_FAILED, ErrorCode.TIMEOUT]

      for (const code of retryableCodes) {
        const error = new BulletinError("test", code)
        expect(error.retryable).toBe(true)
      }
    })

    it("should return false for non-retryable error codes", () => {
      const nonRetryableCodes = [
        ErrorCode.EMPTY_DATA,
        ErrorCode.DATA_TOO_LARGE,
        ErrorCode.CHUNK_TOO_LARGE,
        ErrorCode.INVALID_CHUNK_SIZE,
        ErrorCode.INVALID_CONFIG,
        ErrorCode.INVALID_CID,
        ErrorCode.CID_CALCULATION_FAILED,
        ErrorCode.DAG_ENCODING_FAILED,
        ErrorCode.INSUFFICIENT_AUTHORIZATION,
        ErrorCode.AUTHORIZATION_FAILED,
        ErrorCode.CHUNK_FAILED,
        ErrorCode.MISSING_CHUNK,
        ErrorCode.UNSUPPORTED_OPERATION,
      ]

      for (const code of nonRetryableCodes) {
        const error = new BulletinError("test", code)
        expect(error.retryable).toBe(false)
      }
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
  })

  describe("TransactionStatusEvent variants", () => {
    it("should support validated event", () => {
      const event: TransactionStatusEvent = { type: TxStatus.Validated }
      expect(event.type).toBe(TxStatus.Validated)
    })

    it("should support broadcasted event", () => {
      const event: TransactionStatusEvent = { type: TxStatus.Broadcasted }
      expect(event.type).toBe(TxStatus.Broadcasted)
    })

    it("should support in_block event", () => {
      const event: TransactionStatusEvent = {
        type: TxStatus.InBlock,
        blockHash: "0xabc",
        blockNumber: 42,
        txIndex: 1,
      }
      expect(event.type).toBe(TxStatus.InBlock)
      expect(event.blockHash).toBe("0xabc")
      expect(event.blockNumber).toBe(42)
      expect(event.txIndex).toBe(1)
    })

    it("should support no_longer_in_block event", () => {
      const event: TransactionStatusEvent = { type: TxStatus.NoLongerInBlock }
      expect(event.type).toBe(TxStatus.NoLongerInBlock)
    })

    it("should support invalid event", () => {
      const event: TransactionStatusEvent = {
        type: TxStatus.Invalid,
        error: "nonce too low",
      }
      expect(event.type).toBe(TxStatus.Invalid)
      expect(event.error).toBe("nonce too low")
    })

    it("should support dropped event", () => {
      const event: TransactionStatusEvent = {
        type: TxStatus.Dropped,
        error: "pool full",
      }
      expect(event.type).toBe(TxStatus.Dropped)
      expect(event.error).toBe("pool full")
    })

    it("should remain backward compatible with string comparisons", () => {
      const event: TransactionStatusEvent = {
        type: TxStatus.InBlock,
        blockHash: "0x1",
        blockNumber: 1,
      }
      // biome-ignore lint/suspicious/noExplicitAny: testing backward compat with string comparison
      expect(event.type === ("in_block" as any)).toBe(true)
    })
  })
})
