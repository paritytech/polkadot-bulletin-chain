// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest"
import { AsyncBulletinClient } from "../../src/async-client"

// With waitFor "in_block" the SDK must keep each transaction's PAPI
// subscription alive until finalization: unsubscribing sends
// transaction_v1_stop, and a stopped tx is not re-included after a reorg,
// which strands the next chunk's nonce until its mortality era expires
// (AncientBirthBlock, see the chunked-upload CI failure in #687).

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

describe("chunked upload tx subscription lifecycle", () => {
  it("keeps each chunk watched after in_block and unsubscribes on finalized", async () => {
    const observers: Observer[] = []
    const unsubscribed: boolean[] = []
    const makeTx = () => ({
      signSubmitAndWatch: () => ({
        subscribe: (obs: Observer) => {
          const i = observers.length
          observers.push(obs)
          unsubscribed.push(false)
          return {
            unsubscribe: () => {
              unsubscribed[i] = true
            },
          }
        },
      }),
      getBareTx: async () => new Uint8Array(),
      decodedCall: {},
      signAndSubmit: async () => ({ txHash: "0x01" }),
    })
    const api = {
      tx: {
        TransactionStorage: {
          store: makeTx,
          store_with_cid_config: makeTx,
        },
      },
    }
    const client = new AsyncBulletinClient(
      // biome-ignore lint/suspicious/noExplicitAny: testing with mock objects
      api as any,
      // biome-ignore lint/suspicious/noExplicitAny: testing with mock objects
      signer as any,
      submitFn,
    )

    // 4 bytes with 2-byte chunks: 2 chunk txs + 1 manifest tx, sequential
    const pending = client.storeWithOptions(
      new Uint8Array([1, 2, 3, 4]),
      { waitFor: "in_block" },
      undefined,
      { chunkSize: 2 },
    )

    const inBlock = (i: number) =>
      observers[i].next({
        txHash: `0x0${i}`,
        type: "txBestBlocksState",
        found: true,
        block: { hash: "0xbe57", number: 10 + i, index: 0 },
      })
    const finalized = (i: number) =>
      observers[i].next({
        type: "finalized",
        block: { hash: "0xf1a1", number: 10 + i, index: 0 },
      })

    await vi.waitFor(() => expect(observers).toHaveLength(1))
    inBlock(0)

    // Chunk 1 starts while chunk 0 is only in a best block: chunk 0 must
    // still be watched so a reorg cannot silently drop it.
    await vi.waitFor(() => expect(observers).toHaveLength(2))
    expect(unsubscribed[0]).toBe(false)
    inBlock(1)

    await vi.waitFor(() => expect(observers).toHaveLength(3))
    inBlock(2)

    const result = await pending
    expect(result.chunks?.numChunks).toBe(2)
    expect(unsubscribed).toEqual([false, false, false])

    // Finalization releases each subscription
    finalized(0)
    finalized(1)
    finalized(2)
    expect(unsubscribed).toEqual([true, true, true])
  })
})
