// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

import { InvalidTxError } from "polkadot-api"
import { describe, expect, it, vi } from "vitest"
import { AsyncBulletinClient, type SubmitFn } from "../../src/async-client"

// A reorg can retract a fresh preimage authorization; PAPI then settles
// the pending bare store as Invalid/Payment and never re-checks, even
// though the authorization is re-included. Only that exact shape may be
// retried; every other invalid type must surface.

const signer = {
  publicKey: new Uint8Array(32),
  sign: async () => new Uint8Array(64),
}

const invalidError = (type: string) =>
  new InvalidTxError({ type: "Invalid", value: { type, value: undefined } })

const submitSuccess = {
  ok: true,
  block: { hash: "0x02", number: 1, index: 0 },
  txHash: "0x01",
  events: [],
}

const setup = (submit: SubmitFn) => {
  const makeTx = () => ({
    getBareTx: async () => new Uint8Array([1]),
    decodedCall: {},
  })
  const api = {
    tx: {
      TransactionStorage: { store: makeTx, store_with_cid_config: makeTx },
    },
  }
  return new AsyncBulletinClient(
    // biome-ignore lint/suspicious/noExplicitAny: testing with mock objects
    api as any,
    // biome-ignore lint/suspicious/noExplicitAny: testing with mock objects
    signer as any,
    submit,
  )
}

describe("preimage store Payment retry", () => {
  it("resubmits once on Invalid/Payment, then resolves", async () => {
    const submit = vi
      .fn<SubmitFn>()
      .mockRejectedValueOnce(invalidError("Payment"))
      .mockResolvedValueOnce(submitSuccess)
    const client = setup(submit)

    const result = await client.storeWithPreimageAuth(new Uint8Array([1, 2, 3]))

    expect(result.cid).toBeDefined()
    expect(submit).toHaveBeenCalledTimes(2)
  })

  it("does not retry other invalid types (Future)", async () => {
    const submit = vi.fn<SubmitFn>().mockRejectedValue(invalidError("Future"))
    const client = setup(submit)

    await expect(
      client.storeWithPreimageAuth(new Uint8Array([1, 2, 3])),
    ).rejects.toThrow("Future")
    expect(submit).toHaveBeenCalledTimes(1)
  })
})
