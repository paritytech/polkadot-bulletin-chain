// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Unsubscribing sends transaction_v1_stop: the node drops the transaction,
// and a transaction whose block is reorged out after that is never included
// again. An in_block call therefore resolves at first inclusion but stays
// subscribed until the transaction finalizes or fails, or until txTimeout
// expires.

import { describe, expect, it, vi } from "vitest"
import type { ProgressEvent } from "../../src/types"

interface Observer {
  next: (ev: unknown) => void
  error: (err: unknown) => void
  complete?: () => void
}

interface Subscription {
  observer: Observer
  unsubscribeCount: number
}

const subscriptions: Subscription[] = []
let unsubscribeError: Error | undefined

const makeTx = () => ({
  createSubmitAndWatch: () => ({
    subscribe: (observer: Observer) => {
      const subscription: Subscription = { observer, unsubscribeCount: 0 }
      subscriptions.push(subscription)
      return {
        unsubscribe: () => {
          subscription.unsubscribeCount += 1
          if (unsubscribeError) throw unsubscribeError
        },
      }
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
const inBestBlock = { type: "inBestBlock", txHash: "0x01", block, events: [] }
const finalized = { type: "finalized", txHash: "0x01", block, events: [] }

async function setup(txTimeout?: number) {
  subscriptions.length = 0
  unsubscribeError = undefined
  const { BulletinClient } = await import("../../src/client")
  const client = new BulletinClient({
    descriptor: {},
    // biome-ignore lint/suspicious/noExplicitAny: provider stub
    providers: () => [{} as any],
    // biome-ignore lint/suspicious/noExplicitAny: signer stub
    uploadSigner: { publicKey: new Uint8Array(32) } as any,
    txTimeout,
  })
  const statuses: string[] = []
  const send = (waitFor: "in_block" | "finalized") =>
    client
      .removeExpiredAccountAuthorization("5Grw")
      .withWaitFor(waitFor)
      .withCallback((ev: ProgressEvent) => {
        statuses.push(ev.type)
      })
      .send()
  const subscription = () =>
    vi.waitFor(() => {
      const first = subscriptions[0]
      if (!first) throw new Error("no subscription yet")
      return first
    })
  return { send, subscription, statuses }
}

describe("single-extrinsic subscription lifecycle", () => {
  it("in_block subscription open until finalization", async () => {
    const { send, subscription } = await setup()
    const pending = send("in_block")
    const watched = await subscription()

    watched.observer.next(inBestBlock)
    await expect(pending).resolves.toMatchObject({ blockNumber: 10 })
    expect(watched.unsubscribeCount).toBe(0)

    watched.observer.next(finalized)
    expect(watched.unsubscribeCount).toBe(1)
  })

  it("subscription release at resolution for finalized wait", async () => {
    const { send, subscription } = await setup()
    const pending = send("finalized")
    const watched = await subscription()

    watched.observer.next(inBestBlock)
    expect(watched.unsubscribeCount).toBe(0)
    watched.observer.next(finalized)

    await expect(pending).resolves.toMatchObject({ blockHash: block.hash })
    expect(watched.unsubscribeCount).toBe(1)
  })

  // The call already resolved; a late failure only ends the watch.
  it("silent subscription release on error after in_block resolution", async () => {
    const { send, subscription, statuses } = await setup()
    const pending = send("in_block")
    const watched = await subscription()

    watched.observer.next(inBestBlock)
    await pending
    const statusesAtResolution = [...statuses]

    watched.observer.error(new Error("transaction dropped from the pool"))
    expect(watched.unsubscribeCount).toBe(1)
    expect(statuses).toEqual(statusesAtResolution)
  })

  it("background watch limit at txTimeout", async () => {
    const { send, subscription } = await setup(50)
    const pending = send("in_block")
    const watched = await subscription()

    watched.observer.next(inBestBlock)
    await pending
    expect(watched.unsubscribeCount).toBe(0)

    await vi.waitFor(() => expect(watched.unsubscribeCount).toBe(1))
  })

  // A client destroyed during the background watch closes the transport, and
  // PAPI's unsubscribe then throws "Not connected".
  it("no throw from unsubscribe after transport loss", async () => {
    const { send, subscription } = await setup()
    const pending = send("in_block")
    const watched = await subscription()

    watched.observer.next(inBestBlock)
    await pending
    unsubscribeError = new Error("Not connected")

    expect(() => watched.observer.next(finalized)).not.toThrow()
    expect(watched.unsubscribeCount).toBe(1)
  })
})
