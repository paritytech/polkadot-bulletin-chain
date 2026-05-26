// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, expect, it, vi } from "vitest"
import { BulletinClient } from "../../src/client"

// File-scope mock — captures the destroy() spy so each test can assert
// on it.
const destroySpy = vi.fn()
vi.mock("polkadot-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("polkadot-api")>()
  return {
    ...actual,
    createClient: vi.fn(() => ({
      getTypedApi: () => ({}),
      submitAndWatch: () => ({ subscribe: () => ({ unsubscribe() {} }) }),
      destroy: destroySpy,
    })),
  }
})

describe("BulletinClient.destroy", () => {
  it("tears down the internal PolkadotClient", async () => {
    destroySpy.mockClear()
    const client = new BulletinClient({
      descriptor: {},
      // biome-ignore lint/suspicious/noExplicitAny: provider stub
      providers: () => [{} as any],
    })
    await client.destroy()
    expect(destroySpy).toHaveBeenCalledTimes(1)
  })

  it("is idempotent — second destroy() is a no-op", async () => {
    destroySpy.mockClear()
    const client = new BulletinClient({
      descriptor: {},
      // biome-ignore lint/suspicious/noExplicitAny: provider stub
      providers: () => [{} as any],
    })
    await client.destroy()
    await client.destroy()
    expect(destroySpy).toHaveBeenCalledTimes(1)
  })
})
