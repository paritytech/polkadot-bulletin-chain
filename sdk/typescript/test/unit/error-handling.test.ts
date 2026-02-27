// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, it, expect } from "vitest";
import { BulletinError, HashAlgorithm } from "../../src/types";
import { BulletinClient } from "../../src/client";
import { calculateCid, parseCid, cidFromBytes } from "../../src/utils";

describe("Error Handling", () => {
  describe("BulletinError", () => {
    it("should create error with code", () => {
      const error = new BulletinError("Test error", "TEST_CODE");

      expect(error.message).toBe("Test error");
      expect(error.code).toBe("TEST_CODE");
      expect(error.name).toBe("BulletinError");
      expect(error.cause).toBeUndefined();
    });

    it("should create error with cause", () => {
      const cause = new Error("Original error");
      const error = new BulletinError("Wrapped error", "WRAPPED", cause);

      expect(error.message).toBe("Wrapped error");
      expect(error.code).toBe("WRAPPED");
      expect(error.cause).toBe(cause);
    });

    it("should be instanceof Error", () => {
      const error = new BulletinError("Test", "CODE");

      expect(error).toBeInstanceOf(Error);
      expect(error).toBeInstanceOf(BulletinError);
    });

    it("should preserve stack trace", () => {
      const error = new BulletinError("Test", "CODE");

      expect(error.stack).toBeDefined();
      expect(error.stack).toContain("BulletinError");
    });
  });

  describe("Async Error Propagation", () => {
    it("should propagate BulletinError through async chain", async () => {
      const asyncFunction = async () => {
        throw new BulletinError("Async error", "ASYNC_ERROR");
      };

      await expect(asyncFunction()).rejects.toThrow(BulletinError);
      await expect(asyncFunction()).rejects.toMatchObject({
        code: "ASYNC_ERROR",
        message: "Async error",
      });
    });

    it("should preserve error type through Promise.all", async () => {
      const promises = [
        Promise.resolve(1),
        Promise.reject(new BulletinError("Error in promise", "PROMISE_ERROR")),
        Promise.resolve(3),
      ];

      try {
        await Promise.all(promises);
        expect.fail("Should have thrown");
      } catch (error) {
        expect(error).toBeInstanceOf(BulletinError);
        expect((error as BulletinError).code).toBe("PROMISE_ERROR");
      }
    });

    it("should preserve error type through Promise.allSettled", async () => {
      const promises = [
        Promise.resolve(1),
        Promise.reject(new BulletinError("Error", "SETTLED_ERROR")),
        Promise.resolve(3),
      ];

      const results = await Promise.allSettled(promises);

      expect(results[0].status).toBe("fulfilled");
      expect(results[1].status).toBe("rejected");
      expect(results[2].status).toBe("fulfilled");

      if (results[1].status === "rejected") {
        expect(results[1].reason).toBeInstanceOf(BulletinError);
        expect((results[1].reason as BulletinError).code).toBe("SETTLED_ERROR");
      }
    });
  });

  describe("Client Error Handling", () => {
    it("should throw BulletinError for empty data in prepareStore", async () => {
      const client = new BulletinClient({ endpoint: "ws://localhost:9944" });

      await expect(client.prepareStore(new Uint8Array(0))).rejects.toThrow(
        BulletinError,
      );
      await expect(client.prepareStore(new Uint8Array(0))).rejects.toMatchObject(
        {
          code: "EMPTY_DATA",
        },
      );
    });

    it("should throw BulletinError for empty data in prepareStoreChunked", async () => {
      const client = new BulletinClient({ endpoint: "ws://localhost:9944" });

      await expect(
        client.prepareStoreChunked(new Uint8Array(0)),
      ).rejects.toThrow(BulletinError);
      await expect(
        client.prepareStoreChunked(new Uint8Array(0)),
      ).rejects.toMatchObject({
        code: "EMPTY_DATA",
      });
    });
  });

  describe("CID Error Handling", () => {
    it("should throw BulletinError for invalid CID string", () => {
      expect(() => parseCid("not-a-valid-cid")).toThrow(BulletinError);
      expect(() => parseCid("not-a-valid-cid")).toThrow("Failed to parse CID");
    });

    it("should throw BulletinError for empty CID string", () => {
      expect(() => parseCid("")).toThrow(BulletinError);
    });

    it("should throw BulletinError for invalid CID bytes", () => {
      const invalidBytes = new Uint8Array([0xff, 0xff, 0xff]);
      expect(() => cidFromBytes(invalidBytes)).toThrow(BulletinError);
    });

    it("should throw BulletinError for empty CID bytes", () => {
      expect(() => cidFromBytes(new Uint8Array(0))).toThrow(BulletinError);
    });

    it("should throw BulletinError for unsupported hash algorithm", async () => {
      const data = new Uint8Array([1, 2, 3, 4, 5]);

      // Keccak256 is not fully supported in the SDK
      await expect(
        calculateCid(data, 0x55, HashAlgorithm.Keccak256),
      ).rejects.toThrow(BulletinError);
    });
  });

  describe("Error Message Quality", () => {
    it("should include useful context in error messages", async () => {
      const client = new BulletinClient({ endpoint: "ws://localhost:9944" });

      try {
        await client.prepareStore(new Uint8Array(0));
        expect.fail("Should have thrown");
      } catch (error) {
        expect(error).toBeInstanceOf(BulletinError);
        const bulletinError = error as BulletinError;

        // Error should have meaningful message
        expect(bulletinError.message.length).toBeGreaterThan(10);

        // Error should have a code
        expect(bulletinError.code).toBeDefined();
        expect(bulletinError.code.length).toBeGreaterThan(0);
      }
    });

    it("should include cause when wrapping errors", () => {
      const originalError = new TypeError("Cannot read property of undefined");
      const wrappedError = new BulletinError(
        "Operation failed",
        "OP_FAILED",
        originalError,
      );

      expect(wrappedError.cause).toBe(originalError);
      expect((wrappedError.cause as Error).message).toContain("undefined");
    });
  });
});
