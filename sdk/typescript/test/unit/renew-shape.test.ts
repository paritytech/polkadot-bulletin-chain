// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest"
import {
  AsyncBulletinClient,
  type TransactionRefInput,
  toTransactionRef,
} from "../../src/async-client"
import { ErrorCode } from "../../src/types"

// The renewal extrinsics moved pallet and changed shape across runtimes:
// TransactionStorage.renew({block, index}), then TransactionStorage
// renew({entry}) plus force_renew, now both calls on DataRenewal. PAPI proxies
// are truthy for any name, so the client resolves pallet and shape from
// getStaticApis() compat levels; api objects without it (like these mocks)
// resolve by entry presence. These tests pin that resolution.

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

// Compat levels as getStaticApis() reports them; 0 = the live runtime lacks
// the call.
const staticApis = (levels: {
  dataRenewalRenew?: number
  renew?: number
  forceRenew?: number
}) => ({
  compat: {
    tx: {
      DataRenewal: { renew: { level: levels.dataRenewalRenew ?? 0 } },
      TransactionStorage: {
        renew: { level: levels.renew ?? 1 },
        force_renew: { level: levels.forceRenew ?? 0 },
      },
    },
  },
})

function createClient(
  txPallet: Record<string, unknown>,
  opts: {
    dataRenewal?: Record<string, unknown>
    getStaticApis?: () => Promise<unknown>
  } = {},
) {
  const api = {
    tx: { TransactionStorage: txPallet, DataRenewal: opts.dataRenewal },
    getStaticApis: opts.getStaticApis,
  }
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

  it("rejects when the resolved pallet has no renew entry", async () => {
    const client = createClient({})

    await expect(client.renew(positionInput).send()).rejects.toMatchObject({
      code: ErrorCode.UNSUPPORTED_OPERATION,
      message: "renew is not supported by this runtime",
    })
  })

  it("passes {entry} when force_renew is present without getStaticApis", async () => {
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
})

describe("renewal pallet resolution", () => {
  it("uses DataRenewal with {entry} when present without getStaticApis", async () => {
    let arg: unknown
    const client = createClient(
      {},
      {
        dataRenewal: {
          renew: (a: unknown) => {
            arg = a
            return mockTx
          },
        },
      },
    )

    await client.renew(positionInput).send()
    expect(arg).toEqual({ entry: positionEntry })
  })

  it("uses DataRenewal when compat reports it present", async () => {
    let arg: unknown
    const client = createClient(
      {
        renew: () => {
          throw new Error("wrong pallet")
        },
        force_renew: () => {
          throw new Error("wrong pallet")
        },
      },
      {
        dataRenewal: {
          renew: (a: unknown) => {
            arg = a
            return mockTx
          },
        },
        getStaticApis: async () => staticApis({ dataRenewalRenew: 1 }),
      },
    )

    await client.renew(hashInput).send()
    expect(arg).toEqual({ entry: hashEntry })
  })

  it("falls back to TransactionStorage when compat reports DataRenewal Incompatible", async () => {
    let arg: unknown
    const client = createClient(
      {
        renew: (a: unknown) => {
          arg = a
          return mockTx
        },
        force_renew: () => mockTx,
      },
      {
        dataRenewal: {
          renew: () => {
            throw new Error("wrong pallet")
          },
        },
        getStaticApis: async () => staticApis({ forceRenew: 1 }),
      },
    )

    await client.renew(positionInput).send()
    expect(arg).toEqual({ entry: positionEntry })
  })

  it("resolves the legacy shape when compat reports only TransactionStorage.renew", async () => {
    let arg: unknown
    const client = createClient(
      {
        renew: (a: unknown) => {
          arg = a
          return mockTx
        },
        force_renew: () => mockTx,
      },
      {
        dataRenewal: { renew: () => mockTx },
        getStaticApis: async () => staticApis({}),
      },
    )

    await client.renew(positionInput).send()
    expect(arg).toEqual({ block: 100, index: 5 })
  })

  it("rejects renew when compat reports renewal on neither pallet", async () => {
    // Entries exist on both pallets (real PAPI proxies always do); only the
    // compat levels reveal the runtime has no renewal calls at all.
    let called = false
    const client = createClient(
      {
        renew: () => {
          called = true
          return mockTx
        },
        force_renew: () => mockTx,
      },
      {
        dataRenewal: { renew: () => mockTx },
        getStaticApis: async () => staticApis({ renew: 0 }),
      },
    )

    await expect(client.renew(positionInput).send()).rejects.toMatchObject({
      code: ErrorCode.UNSUPPORTED_OPERATION,
      message: "renew is not supported by this runtime",
    })
    expect(called).toBe(false)
  })

  it("submits force_renew on DataRenewal when resolved there", async () => {
    let arg: unknown
    const client = createClient(
      {
        force_renew: () => {
          throw new Error("wrong pallet")
        },
      },
      {
        dataRenewal: {
          renew: () => mockTx,
          force_renew: (a: unknown) => {
            arg = a
            return mockTx
          },
        },
        getStaticApis: async () => staticApis({ dataRenewalRenew: 1 }),
      },
    )

    await client.forceRenew(positionInput).send()
    expect(arg).toEqual({ entry: positionEntry })
  })
})

describe("probe caching", () => {
  it("probes the runtime once per client across renew and forceRenew calls", async () => {
    let probes = 0
    const client = createClient(
      { renew: () => mockTx, force_renew: () => mockTx },
      {
        getStaticApis: async () => {
          probes++
          return staticApis({ forceRenew: 1 })
        },
      },
    )

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
    const client = createClient(
      { renew: () => mockTx, force_renew: () => mockTx },
      {
        getStaticApis: async () => {
          probes++
          await gate
          return staticApis({ forceRenew: 1 })
        },
      },
    )

    const both = Promise.all([
      client.renew(positionInput).send(),
      client.renew(hashInput).send(),
    ])
    release()
    await both
    expect(probes).toBe(1)
  })

  it("fails fast on a probe failure and retries on the next call", async () => {
    // A transient probe error must not silently pick a shape — the wrong
    // guess submits wrong args with an opaque error — and is not cached.
    let arg: unknown
    let probes = 0
    const client = createClient(
      {
        renew: (a: unknown) => {
          arg = a
          return mockTx
        },
        force_renew: () => mockTx,
      },
      {
        getStaticApis: async () => {
          probes++
          if (probes === 1) throw new Error("transient rpc error")
          return staticApis({ forceRenew: 1 })
        },
      },
    )

    await expect(client.renew(positionInput).send()).rejects.toMatchObject({
      code: ErrorCode.TRANSACTION_FAILED,
      message: expect.stringContaining("probe"),
    })
    expect(arg).toBeUndefined()

    await client.renew(positionInput).send()
    expect(probes).toBe(2)
    expect(arg).toEqual({ entry: positionEntry })
  })
})

describe("forceRenew", () => {
  it("submits force_renew({entry}) on supporting runtimes", async () => {
    let arg: unknown
    const client = createClient({
      renew: () => mockTx,
      force_renew: (a: unknown) => {
        arg = a
        return mockTx
      },
    })

    await client.forceRenew(positionInput).send()
    expect(arg).toEqual({ entry: positionEntry })
  })

  it("rejects with a clear error when compat reports force_renew Incompatible", async () => {
    const client = createClient(
      { renew: () => mockTx, force_renew: () => mockTx },
      {
        dataRenewal: { renew: () => mockTx },
        getStaticApis: async () => staticApis({}),
      },
    )

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
    const client = createClient(
      { renew: () => mockTx, force_renew: () => mockTx },
      {
        getStaticApis: async () => {
          throw new Error("boom")
        },
      },
    )

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
