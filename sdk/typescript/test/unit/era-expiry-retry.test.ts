// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// A transaction pool can lose a broadcast transaction around a reorg, and the
// loss shows up only as mortality era expiry (AncientBirthBlock). An expired
// signature can never be included, so signing a new transaction cannot
// execute the call twice. Every other invalid outcome is unsafe to retry.

import { InvalidTxError } from "polkadot-api"
import { describe, expect, it, vi } from "vitest"
import { type ProgressEvent, TxStatus } from "../../src/types"

interface Observer {
  next: (ev: unknown) => void
  error: (err: unknown) => void
  complete?: () => void
}

const observers: Observer[] = []

const makeTx = () => ({
  createSubmitAndWatch: () => ({
    subscribe: (observer: Observer) => {
      observers.push(observer)
      return { unsubscribe: () => {} }
    },
  }),
  getBareTx: async () => new Uint8Array(),
  decodedCall: {},
})

vi.mock("polkadot-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("polkadot-api")>()
  return {
    ...actual,
    createClient: vi.fn(() => ({
      getTypedApi: () => ({
        tx: {
          TransactionStorage: { remove_expired_account_authorization: makeTx },
        },
      }),
      submitAndWatch: () => ({ subscribe: () => ({ unsubscribe() {} }) }),
      destroy: () => {},
    })),
  }
})

const block = { hash: "0xbe57", number: 10, index: 0 }

const invalidError = (type: string) =>
  new InvalidTxError({ type: "Invalid", value: { type, value: undefined } })

async function setup() {
  observers.length = 0
  const { BulletinClient } = await import("../../src/client")
  const client = new BulletinClient({
    descriptor: {},
    // biome-ignore lint/suspicious/noExplicitAny: provider stub
    providers: () => [{} as any],
    // biome-ignore lint/suspicious/noExplicitAny: signer stub
    uploadSigner: { publicKey: new Uint8Array(32) } as any,
  })
  const statuses: string[] = []
  const pending = client
    .removeExpiredAccountAuthorization("5Grw")
    .withWaitFor("in_block")
    .withCallback((ev: ProgressEvent) => {
      statuses.push(ev.type)
    })
    .send()
  const attemptObserver = (index: number) =>
    vi.waitFor(() => {
      const observer = observers[index]
      if (!observer) throw new Error(`no attempt ${index} yet`)
      return observer
    })
  return { pending, statuses, attemptObserver }
}

describe("mortality era expiry retry", () => {
  it("single re-submission after AncientBirthBlock", async () => {
    const { pending, attemptObserver } = await setup()

    const first = await attemptObserver(0)
    first.error(invalidError("AncientBirthBlock"))
    const second = await attemptObserver(1)
    second.next({ type: "inBestBlock", txHash: "0x02", block })

    await expect(pending).resolves.toMatchObject({ blockNumber: 10 })
    expect(observers).toHaveLength(2)
  })

  it("no re-submission after other invalid types", async () => {
    const { pending, attemptObserver } = await setup()

    const first = await attemptObserver(0)
    first.error(invalidError("BadSigner"))

    await expect(pending).rejects.toThrow("BadSigner")
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(observers).toHaveLength(1)
  })

  it("rejection after second AncientBirthBlock", async () => {
    const { pending, attemptObserver } = await setup()

    const first = await attemptObserver(0)
    first.error(invalidError("AncientBirthBlock"))
    const second = await attemptObserver(1)
    second.error(invalidError("AncientBirthBlock"))

    await expect(pending).rejects.toThrow("AncientBirthBlock")
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(observers).toHaveLength(2)
  })

  // The retried attempt is not a failure of the call.
  it("Invalid progress event only for final attempt", async () => {
    const { pending, statuses, attemptObserver } = await setup()

    const first = await attemptObserver(0)
    first.error(invalidError("AncientBirthBlock"))
    expect(statuses).not.toContain(TxStatus.Invalid)
    const second = await attemptObserver(1)
    second.error(invalidError("AncientBirthBlock"))

    await expect(pending).rejects.toThrow()
    expect(statuses.filter((status) => status === TxStatus.Invalid)).toEqual([
      TxStatus.Invalid,
    ])
  })
})
