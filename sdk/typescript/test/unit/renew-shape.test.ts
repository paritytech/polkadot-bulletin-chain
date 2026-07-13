// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest"
import {
  AsyncBulletinClient,
  type TransactionRef,
} from "../../src/async-client"
import { ErrorCode } from "../../src/types"

// The renewal extrinsics changed shape: old runtimes take `renew({block, index})`,
// current ones take `renew({entry: TransactionRef})` and add `force_renew`.
// These tests pin the client's runtime detection for both shapes.

const positionEntry: TransactionRef = {
  type: "Position",
  value: { block: 100, index: 5 },
}

const hashEntry: TransactionRef = {
  type: "ContentHash",
  value: {
    asBytes: () => new Uint8Array(32),
    asHex: () => `0x${"00".repeat(32)}`,
  },
}

const signer = {
  publicKey: new Uint8Array(32),
  sign: async () => new Uint8Array(64),
}

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

const submitFn = async () => ({
  ok: true,
  block: { hash: "0x02", number: 1, index: 0 },
  txHash: "0x01",
  events: [],
})

function createClient(txPallet: Record<string, unknown>) {
  const api = { tx: { TransactionStorage: txPallet } }
  // biome-ignore lint/suspicious/noExplicitAny: testing with mock objects
  return new AsyncBulletinClient(api as any, signer as any, submitFn)
}

describe("renew argument shape detection", () => {
  it("unpacks Position to {block, index} when the api has no force_renew (old runtime)", async () => {
    let arg: unknown
    const client = createClient({
      renew: (a: unknown) => {
        arg = a
        return mockTx
      },
    })

    await client.renew(positionEntry).send()
    expect(arg).toEqual({ block: 100, index: 5 })
  })

  it("rejects ContentHash entries on old runtimes", async () => {
    const client = createClient({
      renew: () => mockTx,
    })

    await expect(client.renew(hashEntry).send()).rejects.toMatchObject({
      code: ErrorCode.TRANSACTION_FAILED,
      message: "content-hash renewal is not supported by this runtime",
    })
  })

  it("passes {entry} when force_renew exists without a compatibility probe", async () => {
    let arg: unknown
    const client = createClient({
      renew: (a: unknown) => {
        arg = a
        return mockTx
      },
      force_renew: () => mockTx,
    })

    await client.renew(positionEntry).send()
    expect(arg).toEqual({ entry: positionEntry })
  })

  it("unpacks Position when the PAPI probe reports Incompatible", async () => {
    // A real PAPI proxy returns a truthy entry for any name; only
    // getCompatibilityLevel() reveals that the runtime lacks force_renew.
    let arg: unknown
    const forceRenew = () => mockTx
    forceRenew.getCompatibilityLevel = async () => 0
    const client = createClient({
      renew: (a: unknown) => {
        arg = a
        return mockTx
      },
      force_renew: forceRenew,
    })

    await client.renew(positionEntry).send()
    expect(arg).toEqual({ block: 100, index: 5 })
  })

  it("passes {entry} when the PAPI probe reports compatible", async () => {
    let arg: unknown
    const forceRenew = () => mockTx
    forceRenew.getCompatibilityLevel = async () => 3
    const client = createClient({
      renew: (a: unknown) => {
        arg = a
        return mockTx
      },
      force_renew: forceRenew,
    })

    await client.renew(hashEntry).send()
    expect(arg).toEqual({ entry: hashEntry })
  })

  it("treats a throwing probe as unsupported", async () => {
    let arg: unknown
    const forceRenew = () => mockTx
    forceRenew.getCompatibilityLevel = async () => {
      throw new Error("no entry point")
    }
    const client = createClient({
      renew: (a: unknown) => {
        arg = a
        return mockTx
      },
      force_renew: forceRenew,
    })

    await client.renew(positionEntry).send()
    expect(arg).toEqual({ block: 100, index: 5 })
  })
})

describe("forceRenew", () => {
  it("submits force_renew({entry}) on supporting runtimes", async () => {
    let arg: unknown
    const forceRenew = (a: unknown) => {
      arg = a
      return mockTx
    }
    forceRenew.getCompatibilityLevel = async () => 2
    const client = createClient({
      renew: () => mockTx,
      force_renew: forceRenew,
    })

    await client.forceRenew(positionEntry).send()
    expect(arg).toEqual({ entry: positionEntry })
  })

  it("rejects with a clear error when the runtime lacks force_renew", async () => {
    const forceRenew = () => mockTx
    forceRenew.getCompatibilityLevel = async () => 0
    const client = createClient({
      renew: () => mockTx,
      force_renew: forceRenew,
    })

    await expect(client.forceRenew(positionEntry).send()).rejects.toMatchObject(
      {
        code: ErrorCode.TRANSACTION_FAILED,
        message: "force_renew is not supported by this runtime",
      },
    )
  })
})
