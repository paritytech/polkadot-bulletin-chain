// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

import { InvalidTxError } from "polkadot-api"
import { describe, expect, it, vi } from "vitest"
import { AsyncBulletinClient } from "../../src/async-client"

// The node's fork-aware pool can silently lose a broadcast tx around a
// reorg; PAPI reports it as InvalidTxError/AncientBirthBlock once the
// 64-block era boundary finalizes. The SDK must re-sign and resubmit once
// (safe: the expired signature can never be included), and must NOT retry
// other invalid types.

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

const invalidError = (type: string) =>
  new InvalidTxError({ type: "Invalid", value: { type, value: undefined } })

const setup = () => {
  const observers: Observer[] = []
  const makeTx = () => ({
    signSubmitAndWatch: () => ({
      subscribe: (obs: Observer) => {
        observers.push(obs)
        return { unsubscribe: () => {} }
      },
    }),
    getBareTx: async () => new Uint8Array(),
    decodedCall: {},
    signAndSubmit: async () => ({ txHash: "0x01" }),
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
  return { observers, client }
}

describe("mortality era expiry retry", () => {
  it("re-signs and resubmits once on AncientBirthBlock, then resolves", async () => {
    const { observers, client } = setup()

    const pending = client.storeWithOptions(new Uint8Array([1, 2, 3]), {
      waitFor: "in_block",
    })

    await vi.waitFor(() => expect(observers).toHaveLength(1))
    observers[0].error(invalidError("AncientBirthBlock"))

    // The retry re-invokes the full sign-submit-watch cycle
    await vi.waitFor(() => expect(observers).toHaveLength(2))
    observers[1].next({
      txHash: "0x01",
      type: "txBestBlocksState",
      found: true,
      block: { hash: "0xbe57", number: 10, index: 0 },
    })

    const result = await pending
    expect(result.cid).toBeDefined()
    expect(observers).toHaveLength(2)

    // Release the background finalization watch
    observers[1].next({
      type: "finalized",
      block: { hash: "0xf1a1", number: 10, index: 0 },
    })
  })

  it("does not retry other invalid types (BadSigner)", async () => {
    const { observers, client } = setup()

    const pending = client.storeWithOptions(new Uint8Array([1, 2, 3]), {
      waitFor: "in_block",
    })

    await vi.waitFor(() => expect(observers).toHaveLength(1))
    observers[0].error(invalidError("BadSigner"))

    await expect(pending).rejects.toThrow("BadSigner")
    // Give a hypothetical retry a chance to subscribe before asserting
    await new Promise((r) => setTimeout(r, 0))
    expect(observers).toHaveLength(1)
  })
})
