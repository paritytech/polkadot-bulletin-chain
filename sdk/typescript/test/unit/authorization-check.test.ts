// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Tests for the opt-in `ensureAuthorized()` pre-flight on the upload
 * builders. The chain itself does not reject store calls when allowance
 * is exhausted (it lowers priority via `AllowanceBasedPriority`), so the
 * SDK's pre-flight only verifies that an `Authorizations` entry exists
 * and is not expired.
 */

import { Binary } from "polkadot-api"
import { describe, expect, it } from "vitest"
import { MockBulletinClient } from "../../src/mock-client"
import { BulletinError, ErrorCode } from "../../src/types"

describe("ensureAuthorized() pre-flight", () => {
  describe("MockBulletinClient", () => {
    it("throws INSUFFICIENT_AUTHORIZATION when simulateInsufficientAuth + ensureAuthorized()", async () => {
      const client = new MockBulletinClient({
        simulateInsufficientAuth: true,
      })
      await expect(
        client.uploadFile(Binary.fromText("hello")).ensureAuthorized().send(),
      ).rejects.toMatchObject({
        code: ErrorCode.INSUFFICIENT_AUTHORIZATION,
      })
    })

    it("does NOT throw when simulateInsufficientAuth + NO ensureAuthorized()", async () => {
      const client = new MockBulletinClient({
        simulateInsufficientAuth: true,
      })
      const result = await client.uploadFile(Binary.fromText("hello")).send()
      expect(result.cid).toBeDefined()
    })

    it("passes when ensureAuthorized() + no simulated failure", async () => {
      const client = new MockBulletinClient()
      const result = await client
        .uploadFile(Binary.fromText("hello"))
        .ensureAuthorized()
        .send()
      expect(result.cid).toBeDefined()
    })
  })

  describe("BulletinClient.ensureAuthorizedOnChain (via reflection)", () => {
    // Direct testing of the private method via `as any`. The method only
    // touches `this.api.query` and `this.signer.publicKey`; we can call it
    // in isolation without needing a real chainHead follow.
    async function callEnsureAuthorized(
      query: unknown,
      systemNumber?: number,
    ): Promise<void> {
      const { BulletinClient } = await import("../../src/client")

      const sysQuery =
        systemNumber !== undefined
          ? { System: { Number: { getValue: async () => systemNumber } } }
          : {}

      const apiStub = {
        tx: { TransactionStorage: {}, Sudo: { sudo: () => ({}) } },
        query:
          query === null
            ? undefined
            : Object.assign({}, query as object, sysQuery),
      }
      const signer = {
        publicKey: new Uint8Array(32),
        sign: async () => new Uint8Array(64),
      }
      // biome-ignore lint/suspicious/noExplicitAny: tests touch private method directly
      const client = new BulletinClient(
        apiStub as any,
        signer as any,
        undefined,
      )
      // biome-ignore lint/suspicious/noExplicitAny: invoking private method by name
      await (client as any).ensureAuthorizedOnChain()
    }

    it("throws UNSUPPORTED_OPERATION when api.query is unavailable", async () => {
      await expect(callEnsureAuthorized(null)).rejects.toMatchObject({
        code: ErrorCode.UNSUPPORTED_OPERATION,
      })
    })

    it("throws INSUFFICIENT_AUTHORIZATION when no Authorizations entry exists", async () => {
      await expect(
        callEnsureAuthorized({
          TransactionStorage: {
            Authorizations: { getValue: async () => undefined },
          },
        }),
      ).rejects.toMatchObject({
        code: ErrorCode.INSUFFICIENT_AUTHORIZATION,
      })
    })

    it("throws INSUFFICIENT_AUTHORIZATION when authorization has expired", async () => {
      await expect(
        callEnsureAuthorized(
          {
            TransactionStorage: {
              Authorizations: {
                getValue: async () => ({
                  extent: { transactions: 0, bytes: 0n },
                  expiration: 100,
                }),
              },
            },
          },
          200, // current block > expiration
        ),
      ).rejects.toMatchObject({
        code: ErrorCode.INSUFFICIENT_AUTHORIZATION,
        message: expect.stringMatching(/expired/i),
      })
    })

    it("passes when authorization exists and is not expired", async () => {
      await expect(
        callEnsureAuthorized(
          {
            TransactionStorage: {
              Authorizations: {
                getValue: async () => ({
                  extent: { transactions: 0, bytes: 0n },
                  expiration: 1_000_000,
                }),
              },
            },
          },
          100, // current block < expiration
        ),
      ).resolves.toBeUndefined()
    })

    it("passes when expiration is unknown (no System.Number on api.query)", async () => {
      // Without System.Number we can't tell if it's expired; pre-flight
      // accepts and lets the chain reject if it's actually expired.
      await expect(
        callEnsureAuthorized({
          TransactionStorage: {
            Authorizations: {
              getValue: async () => ({
                extent: { transactions: 0, bytes: 0n },
                expiration: 100,
              }),
            },
          },
        }),
      ).resolves.toBeUndefined()
    })

    it("returned error is a BulletinError with recoveryHint", async () => {
      try {
        await callEnsureAuthorized({
          TransactionStorage: {
            Authorizations: { getValue: async () => undefined },
          },
        })
        expect.fail("Should have thrown")
      } catch (error) {
        expect(error).toBeInstanceOf(BulletinError)
        expect((error as BulletinError).code).toBe(
          ErrorCode.INSUFFICIENT_AUTHORIZATION,
        )
        expect((error as BulletinError).recoveryHint).toContain(
          "authorizeAccount()",
        )
      }
    })
  })
})
