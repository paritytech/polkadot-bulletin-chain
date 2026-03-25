// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { Binary } from "polkadot-api"
import { describe, expect, it } from "vitest"
import { MockBulletinClient } from "../../src/mock-client"
import { BulletinError, ErrorCode } from "../../src/types"

describe("Authorization Check", () => {
  describe("MockBulletinClient simulateInsufficientAuth", () => {
    it("should throw INSUFFICIENT_AUTHORIZATION when simulateInsufficientAuth is true", async () => {
      const client = new MockBulletinClient({
        simulateInsufficientAuth: true,
      })

      await expect(
        client.store(Binary.fromText("hello")).send(),
      ).rejects.toMatchObject({
        code: ErrorCode.INSUFFICIENT_AUTHORIZATION,
      })
    })

    it("should not throw when simulateInsufficientAuth is false", async () => {
      const client = new MockBulletinClient({
        simulateInsufficientAuth: false,
      })

      const result = await client.store(Binary.fromText("hello")).send()
      expect(result.cid).toBeDefined()
    })
  })

  describe("AsyncBulletinClient authorization check", () => {
    // These tests exercise the checkAccountAuthorization logic through
    // the AsyncBulletinClient by providing mock api.query implementations.

    // We dynamically import to avoid pulling in full PAPI at module level.
    async function createClientWithQuery(queryImpl: unknown) {
      const { AsyncBulletinClient } = await import("../../src/async-client")

      // Minimal mock signer
      const signer = {
        publicKey: new Uint8Array(32), // all zeros
        sign: async () => new Uint8Array(64),
      }

      // Minimal mock tx object
      const mockTx = {
        signAndSubmit: async () => ({
          txHash: "0x01",
          block: { hash: "0x02", number: 1 },
        }),
        signSubmitAndWatch: () => ({
          subscribe: (observer: {
            next: (ev: unknown) => void
            error: (err: unknown) => void
          }) => {
            // Defer so signAndSubmitWithProgress's timerId is initialized
            setTimeout(() => {
              observer.next({
                txHash: "0x01",
                type: "finalized",
                block: { hash: "0x02", number: 1 },
              })
            }, 0)
            return { unsubscribe: () => {} }
          },
        }),
        getBareTx: async () => "0x00",
        decodedCall: {},
      }

      const api = {
        tx: {
          TransactionStorage: {
            store: () => mockTx,
            store_with_cid_config: () => mockTx,
            authorize_account: () => mockTx,
            authorize_preimage: () => mockTx,
            renew: () => mockTx,
            remove_expired_account_authorization: () => mockTx,
            remove_expired_preimage_authorization: () => mockTx,
            refresh_account_authorization: () => mockTx,
            refresh_preimage_authorization: () => mockTx,
          },
        },
        query: queryImpl,
      }

      const submitFn = async () => ({
        ok: true,
        block: { hash: "0x02", number: 1, index: 0 },
        txHash: "0x01",
        events: [],
      })

      // biome-ignore lint/suspicious/noExplicitAny: testing with mock objects
      return new AsyncBulletinClient(api as any, signer as any, submitFn)
    }

    it("should skip check gracefully when api.query is not provided", async () => {
      const client = await createClientWithQuery(undefined)

      // Should not throw — check is skipped
      const result = await client
        .store(Binary.fromText("hello"))
        .withWaitFor("in_block")
        .send()
      expect(result.cid).toBeDefined()
    })

    it("should propagate network errors from query", async () => {
      const client = await createClientWithQuery({
        TransactionStorage: {
          Authorizations: {
            getValue: async () => {
              throw new Error("Network timeout")
            },
          },
        },
      })

      await expect(
        client.store(Binary.fromText("hello")).withWaitFor("in_block").send(),
      ).rejects.toThrow("Network timeout")
    })

    it("should throw INSUFFICIENT_AUTHORIZATION when no authorization exists", async () => {
      const client = await createClientWithQuery({
        TransactionStorage: {
          Authorizations: {
            getValue: async () => undefined,
          },
        },
      })

      await expect(
        client.store(Binary.fromText("hello")).withWaitFor("in_block").send(),
      ).rejects.toMatchObject({
        code: ErrorCode.INSUFFICIENT_AUTHORIZATION,
      })
    })

    it("should throw INSUFFICIENT_AUTHORIZATION when transactions insufficient", async () => {
      const client = await createClientWithQuery({
        TransactionStorage: {
          Authorizations: {
            getValue: async () => ({
              extent: { transactions: 0, bytes: BigInt(1000000) },
              expiration: 999999,
            }),
          },
        },
      })

      await expect(
        client.store(Binary.fromText("hello")).withWaitFor("in_block").send(),
      ).rejects.toMatchObject({
        code: ErrorCode.INSUFFICIENT_AUTHORIZATION,
        message: expect.stringContaining("transactions"),
      })
    })

    it("should throw INSUFFICIENT_AUTHORIZATION when bytes insufficient", async () => {
      const client = await createClientWithQuery({
        TransactionStorage: {
          Authorizations: {
            getValue: async () => ({
              extent: { transactions: 10, bytes: BigInt(1) },
              expiration: 999999,
            }),
          },
        },
      })

      await expect(
        client
          .store(Binary.fromText("hello world, this is longer than 1 byte"))
          .withWaitFor("in_block")
          .send(),
      ).rejects.toMatchObject({
        code: ErrorCode.INSUFFICIENT_AUTHORIZATION,
        message: expect.stringContaining("bytes"),
      })
    })

    it("should pass when authorization is sufficient", async () => {
      const client = await createClientWithQuery({
        TransactionStorage: {
          Authorizations: {
            getValue: async () => ({
              extent: { transactions: 10, bytes: BigInt(1000000) },
              expiration: 999999,
            }),
          },
        },
      })

      const result = await client
        .store(Binary.fromText("hello"))
        .withWaitFor("in_block")
        .send()
      expect(result.cid).toBeDefined()
    })

    it("should verify error is BulletinError instance", async () => {
      const client = await createClientWithQuery({
        TransactionStorage: {
          Authorizations: {
            getValue: async () => undefined,
          },
        },
      })

      try {
        await client
          .store(Binary.fromText("hello"))
          .withWaitFor("in_block")
          .send()
        expect.fail("Should have thrown")
      } catch (error) {
        expect(error).toBeInstanceOf(BulletinError)
        expect((error as BulletinError).recoveryHint).toContain(
          "authorizeAccount()",
        )
      }
    })
  })
})
