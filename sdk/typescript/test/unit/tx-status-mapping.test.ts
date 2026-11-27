// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest"
import { AsyncBulletinClient } from "../../src/async-client"
import { type ProgressEvent, TxStatus } from "../../src/types"

interface Observer {
  next: (ev: unknown) => void
  error: (err: unknown) => void
  complete?: () => void
}

const signer = {
  publicKey: new Uint8Array(32),
  sign: async () => new Uint8Array(64),
}

const submitFn = async () => ({
  ok: true,
  block: { hash: "0x02", number: 1, index: 0 },
  txHash: "0x01",
  events: [],
})

const block = { hash: "0xbe57", number: 10, index: 0 }

const setup = () => {
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
  const api = {
    tx: {
      TransactionStorage: { store: makeTx, store_with_cid_config: makeTx },
    },
  }
  const client = new AsyncBulletinClient(
    // biome-ignore lint/suspicious/noExplicitAny: testing with mock objects
    api as any,
    // biome-ignore lint/suspicious/noExplicitAny: testing with mock objects
    signer as any,
    submitFn,
  )
  const statuses: string[] = []
  const onProgress = (ev: ProgressEvent) => {
    statuses.push(ev.type)
  }
  return { observers, client, statuses, onProgress }
}

describe("PAPI tx event to progress mapping", () => {
  it("maps every event type when waiting for finalization", async () => {
    const { observers, client, statuses, onProgress } = setup()

    const pending = client.storeWithOptions(
      new Uint8Array([1, 2, 3]),
      { waitFor: "finalized" },
      onProgress,
    )

    await vi.waitFor(() => expect(observers).toHaveLength(1))
    const [obs] = observers
    obs.next({ type: "created", txHash: "0x01" })
    obs.next({ type: "broadcasted", txHash: "0x01" })
    obs.next({ type: "notInBestBlock", txHash: "0x01" })
    obs.next({ type: "inBestBlock", txHash: "0x01", block, events: [] })
    obs.next({ type: "finalized", txHash: "0x01", block, events: [] })

    await expect(pending).resolves.toBeDefined()
    expect(statuses).toEqual([
      TxStatus.Created,
      TxStatus.Broadcasted,
      TxStatus.NoLongerInBlock,
      TxStatus.InBlock,
      TxStatus.Finalized,
    ])
  })

  it("resolves at inBestBlock for in_block and reports nothing afterwards", async () => {
    const { observers, client, statuses, onProgress } = setup()

    const pending = client.storeWithOptions(
      new Uint8Array([1, 2, 3]),
      { waitFor: "in_block" },
      onProgress,
    )

    await vi.waitFor(() => expect(observers).toHaveLength(1))
    const [obs] = observers
    obs.next({ type: "created", txHash: "0x01" })
    obs.next({ type: "broadcasted", txHash: "0x01" })
    obs.next({ type: "inBestBlock", txHash: "0x01", block, events: [] })

    const result = await pending
    expect(result.blockNumber).toBe(block.number)

    obs.next({ type: "finalized", txHash: "0x01", block, events: [] })
    expect(statuses).toEqual([
      TxStatus.Created,
      TxStatus.Broadcasted,
      TxStatus.InBlock,
    ])
  })
})
