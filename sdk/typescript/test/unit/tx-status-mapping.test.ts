// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

import { InvalidTxError } from "polkadot-api"
import { describe, expect, it, vi } from "vitest"
import { ErrorCode, type ProgressEvent, TxStatus } from "../../src/types"

interface Observer {
  next: (ev: unknown) => void
  error: (err: unknown) => void
  complete?: () => void
}

const observers: Observer[] = []
const makeTx = () => ({
  createSubmitAndWatch: () => ({
    subscribe: (obs: Observer) => {
      observers.push(obs)
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

// Single-extrinsic calls (authorize, renew, remove-expired) report progress
// through the PAPI event stream; uploads use the pipeline's UploadStatus.
async function setup(config: Record<string, unknown> = {}) {
  observers.length = 0
  const { BulletinClient } = await import("../../src/client")
  const client = new BulletinClient({
    descriptor: {},
    // biome-ignore lint/suspicious/noExplicitAny: provider stub
    providers: () => [{} as any],
    // biome-ignore lint/suspicious/noExplicitAny: signer stub
    uploadSigner: { publicKey: new Uint8Array(32) } as any,
    ...config,
  })
  const statuses: string[] = []
  const onProgress = (ev: ProgressEvent) => {
    statuses.push(ev.type)
  }
  return { client, statuses, onProgress }
}

describe("PAPI tx event to progress mapping", () => {
  it("maps every event type when waiting for finalization", async () => {
    const { client, statuses, onProgress } = await setup()

    const pending = client
      .removeExpiredAccountAuthorization("5Grw")
      .withWaitFor("finalized")
      .withCallback(onProgress)
      .send()

    const observer = await vi.waitFor(() => {
      const first = observers[0]
      if (!first) throw new Error("no subscription yet")
      return first
    })
    observer.next({ type: "created", txHash: "0x01" })
    observer.next({ type: "broadcasted", txHash: "0x01" })
    observer.next({ type: "notInBestBlock", txHash: "0x01" })
    observer.next({ type: "inBestBlock", txHash: "0x01", block, events: [] })
    observer.next({ type: "finalized", txHash: "0x01", block, events: [] })

    await expect(pending).resolves.toMatchObject({ blockHash: block.hash })
    expect(statuses).toEqual([
      TxStatus.Created,
      TxStatus.Broadcasted,
      TxStatus.NoLongerInBlock,
      TxStatus.InBlock,
      TxStatus.Finalized,
    ])
  })

  it("resolves at inBestBlock for in_block and reports nothing afterwards", async () => {
    const { client, statuses, onProgress } = await setup()

    const pending = client
      .removeExpiredAccountAuthorization("5Grw")
      .withWaitFor("in_block")
      .withCallback(onProgress)
      .send()

    const observer = await vi.waitFor(() => {
      const first = observers[0]
      if (!first) throw new Error("no subscription yet")
      return first
    })
    observer.next({ type: "created", txHash: "0x01" })
    observer.next({ type: "broadcasted", txHash: "0x01" })
    observer.next({ type: "inBestBlock", txHash: "0x01", block, events: [] })

    const result = await pending
    expect(result.blockNumber).toBe(block.number)

    observer.next({ type: "finalized", txHash: "0x01", block, events: [] })
    expect(statuses).toEqual([
      TxStatus.Created,
      TxStatus.Broadcasted,
      TxStatus.InBlock,
    ])
  })
})

describe("default wait level of single-extrinsic calls", () => {
  const firstObserver = () =>
    vi.waitFor(() => {
      const first = observers[0]
      if (!first) throw new Error("no subscription yet")
      return first
    })

  it("finalized without a client waitFor", async () => {
    const { client } = await setup()
    let settled = false
    const pending = client
      .removeExpiredAccountAuthorization("5Grw")
      .send()
      .finally(() => {
        settled = true
      })

    const observer = await firstObserver()
    observer.next({ type: "inBestBlock", txHash: "0x01", block, events: [] })
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(settled).toBe(false)

    observer.next({ type: "finalized", txHash: "0x01", block, events: [] })
    await expect(pending).resolves.toMatchObject({ blockHash: block.hash })
  })

  it("in-block with client waitFor in_block", async () => {
    const { client } = await setup({ waitFor: "in_block" })
    const pending = client.removeExpiredAccountAuthorization("5Grw").send()

    const observer = await firstObserver()
    observer.next({ type: "inBestBlock", txHash: "0x01", block, events: [] })
    await expect(pending).resolves.toMatchObject({ blockHash: block.hash })
  })
})

describe("submission failure codes", () => {
  const firstObserver = () =>
    vi.waitFor(() => {
      const first = observers[0]
      if (!first) throw new Error("no subscription yet")
      return first
    })

  it("pool rejection as TRANSACTION_INVALID", async () => {
    const { client } = await setup()
    const pending = client.removeExpiredAccountAuthorization("5Grw").send()

    const rejection = new InvalidTxError({
      type: "Invalid",
      value: { type: "Custom", value: 11 },
    })
    const observer = await firstObserver()
    observer.error(rejection)

    await expect(pending).rejects.toMatchObject({
      code: ErrorCode.TRANSACTION_INVALID,
      cause: rejection,
    })
  })

  it("other failure as TRANSACTION_FAILED", async () => {
    const { client } = await setup()
    const pending = client.removeExpiredAccountAuthorization("5Grw").send()

    const observer = await firstObserver()
    observer.error(new Error("connection closed"))

    await expect(pending).rejects.toMatchObject({
      code: ErrorCode.TRANSACTION_FAILED,
    })
  })
})
