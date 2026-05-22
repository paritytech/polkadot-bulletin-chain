// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, expect, it, vi } from "vitest"
import { BulletinClient } from "../../src/client"

// Minimal stand-ins; destroy() doesn't touch any of these.
const dummyApi = {} as never
const dummySigner = {} as never

describe("BulletinClient.destroy", () => {
  it("resolves to a no-op when no onDestroy is provided", async () => {
    const client = new BulletinClient(dummyApi, dummySigner, undefined)
    await expect(client.destroy()).resolves.toBeUndefined()
  })

  it("invokes onDestroy and awaits async teardown", async () => {
    const teardown = vi.fn(
      () => new Promise<void>((resolve) => setTimeout(resolve, 5)),
    )
    const client = new BulletinClient(
      dummyApi,
      dummySigner,
      undefined,
      undefined,
      teardown,
    )

    await client.destroy()

    expect(teardown).toHaveBeenCalledTimes(1)
  })
})
