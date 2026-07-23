// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest"
import {
  AsyncBulletinClient,
  type TransactionRefInput,
  toTransactionRef,
} from "../../src/async-client"
import { ErrorCode } from "../../src/types"

// The renewal extrinsics changed shape: old runtimes take `renew({block, index})`,
// current ones take `renew({entry: TransactionRef})` and add `force_renew`.
// These tests pin the client's runtime detection for both shapes.

const positionInput: TransactionRefInput = { block: 100, index: 5 }
const positionEntry = toTransactionRef(positionInput)

const hashInput: TransactionRefInput = new Uint8Array(32).fill(1)
const hashEntry = toTransactionRef(hashInput)

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

    await client.renew(positionInput).send()
    expect(arg).toEqual({ block: 100, index: 5 })
  })

  it("rejects ContentHash entries on old runtimes", async () => {
    const client = createClient({
      renew: () => mockTx,
    })

    await expect(client.renew(hashInput).send()).rejects.toMatchObject({
      code: ErrorCode.UNSUPPORTED_OPERATION,
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

    await client.renew(positionInput).send()
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

    await client.renew(positionInput).send()
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

    await client.renew(hashInput).send()
    expect(arg).toEqual({ entry: hashEntry })
  })

  it("fails fast when the probe throws instead of guessing the shape", async () => {
    // A transient RPC error must not silently pick the legacy shape: against
    // a TransactionRef runtime that submits wrong args with an opaque error.
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

    await expect(client.renew(positionInput).send()).rejects.toMatchObject({
      code: ErrorCode.TRANSACTION_FAILED,
      message: expect.stringContaining("probe"),
    })
    expect(arg).toBeUndefined()
  })
})

describe("probe caching", () => {
  it("probes the runtime once per client across renew and forceRenew calls", async () => {
    let probes = 0
    const forceRenew = () => mockTx
    forceRenew.getCompatibilityLevel = async () => {
      probes++
      return 3
    }
    const client = createClient({
      renew: () => mockTx,
      force_renew: forceRenew,
    })

    await client.renew(positionInput).send()
    await client.renew(hashInput).send()
    await client.forceRenew(positionInput).send()
    expect(probes).toBe(1)
  })

  it("deduplicates concurrent first probes", async () => {
    let probes = 0
    let release!: () => void
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const forceRenew = () => mockTx
    forceRenew.getCompatibilityLevel = async () => {
      probes++
      await gate
      return 3
    }
    const client = createClient({
      renew: () => mockTx,
      force_renew: forceRenew,
    })

    const both = Promise.all([
      client.renew(positionInput).send(),
      client.renew(hashInput).send(),
    ])
    release()
    await both
    expect(probes).toBe(1)
  })

  it("retries the probe on the next call after a failure", async () => {
    let arg: unknown
    let probes = 0
    const forceRenew = () => mockTx
    forceRenew.getCompatibilityLevel = async () => {
      probes++
      if (probes === 1) throw new Error("transient rpc error")
      return 3
    }
    const client = createClient({
      renew: (a: unknown) => {
        arg = a
        return mockTx
      },
      force_renew: forceRenew,
    })

    await expect(client.renew(positionInput).send()).rejects.toMatchObject({
      code: ErrorCode.TRANSACTION_FAILED,
    })
    await client.renew(positionInput).send()
    expect(probes).toBe(2)
    expect(arg).toEqual({ entry: positionEntry })
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

    await client.forceRenew(positionInput).send()
    expect(arg).toEqual({ entry: positionEntry })
  })

  it("rejects with a clear error when the runtime lacks force_renew", async () => {
    const forceRenew = () => mockTx
    forceRenew.getCompatibilityLevel = async () => 0
    const client = createClient({
      renew: () => mockTx,
      force_renew: forceRenew,
    })

    await expect(client.forceRenew(positionInput).send()).rejects.toMatchObject(
      {
        code: ErrorCode.UNSUPPORTED_OPERATION,
        message: "force_renew is not supported by this runtime",
      },
    )
  })

  it("rejects when the api has no force_renew entry at all (old runtime)", async () => {
    const client = createClient({
      renew: () => mockTx,
    })

    await expect(client.forceRenew(positionInput).send()).rejects.toMatchObject(
      {
        code: ErrorCode.UNSUPPORTED_OPERATION,
        message: "force_renew is not supported by this runtime",
      },
    )
  })

  it("surfaces a probe failure rather than reporting force_renew unsupported", async () => {
    const forceRenew = () => mockTx
    forceRenew.getCompatibilityLevel = async () => {
      throw new Error("boom")
    }
    const client = createClient({
      renew: () => mockTx,
      force_renew: forceRenew,
    })

    await expect(client.forceRenew(positionInput).send()).rejects.toMatchObject(
      {
        code: ErrorCode.TRANSACTION_FAILED,
        message: expect.stringContaining("probe"),
      },
    )
  })
})

describe("toTransactionRef variant inference", () => {
  it("maps {block, index} to Position", () => {
    expect(toTransactionRef({ block: 7, index: 2 })).toEqual({
      type: "Position",
      value: { block: 7, index: 2 },
    })
  })

  it("maps a Uint8Array to a hex ContentHash", () => {
    // PAPI encodes fixed-size binaries from hex strings (SizedHex), so the
    // converter must emit hex, not raw bytes.
    const hash = new Uint8Array(32).fill(1)
    expect(toTransactionRef(hash)).toEqual({
      type: "ContentHash",
      value: `0x${"01".repeat(32)}`,
    })
  })
})
