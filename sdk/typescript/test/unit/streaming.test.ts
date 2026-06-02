// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, expect, it } from "vitest"
import {
  blobFromBytes,
  blobFromFactory,
  collectBlob,
} from "../../src/blob-source"
import { chunkStream, FixedSizeChunker } from "../../src/chunker"
import { MockBulletinClient } from "../../src/mock-client"
import { BulletinPreparer } from "../../src/preparer"
import { BulletinError, ErrorCode } from "../../src/types"

const MiB = 1024 * 1024

// Deterministic non-repeating bytes so chunks are distinct (no content collisions).
function makeData(size: number): Uint8Array {
  const buf = new Uint8Array(size)
  let x = 0x9e3779b9
  for (let i = 0; i < size; i++) {
    x ^= x << 13
    x ^= x >>> 17
    x ^= x << 5
    x >>>= 0
    buf[i] = x & 0xff
  }
  return buf
}

// A source that yields `data` in arbitrary-sized parts, to stress the re-chunker.
function blobFromParts(
  parts: Uint8Array[],
): ReturnType<typeof blobFromFactory> {
  return blobFromFactory(async function* () {
    for (const p of parts) yield p
  })
}

describe("BlobSource", () => {
  it("collectBlob round-trips blobFromBytes", async () => {
    const data = makeData(1234)
    expect(await collectBlob(blobFromBytes(data))).toEqual(data)
  })

  it("collectBlob concatenates multi-part factory sources", async () => {
    const data = makeData(5000)
    const parts = [
      data.subarray(0, 1000),
      data.subarray(1000, 4096),
      data.subarray(4096),
    ]
    expect(await collectBlob(blobFromParts(parts))).toEqual(data)
  })

  it("open() is re-callable and yields the same bytes", async () => {
    const src = blobFromBytes(makeData(2048))
    expect(await collectBlob(src)).toEqual(await collectBlob(src))
  })
})

describe("chunkStream", () => {
  it("re-slices across arbitrary input boundaries into fixed chunks", async () => {
    const data = makeData(3 * MiB + 777)
    // Pathological part boundaries: tiny, huge, off-by-one.
    const parts = [
      data.subarray(0, 1),
      data.subarray(1, MiB + 5),
      data.subarray(MiB + 5, 3 * MiB),
      data.subarray(3 * MiB),
    ]
    const chunks: Uint8Array[] = []
    for await (const { index, data: c } of chunkStream(
      blobFromParts(parts),
      MiB,
    )) {
      expect(index).toBe(chunks.length)
      chunks.push(c)
    }
    // 3 full MiB chunks + one 777-byte remainder.
    expect(chunks.length).toBe(4)
    expect(chunks.slice(0, 3).every((c) => c.length === MiB)).toBe(true)
    expect(chunks[3]!.length).toBe(777)
    // Reassembling yields the original bytes.
    const out = new Uint8Array(data.length)
    let off = 0
    for (const c of chunks) {
      out.set(c, off)
      off += c.length
    }
    expect(out).toEqual(data)
  })

  it("matches FixedSizeChunker boundaries for the same data", async () => {
    const data = makeData(2 * MiB + 123)
    const fixed = new FixedSizeChunker({ chunkSize: MiB }).chunk(data)
    const streamed: Uint8Array[] = []
    for await (const { data: c } of chunkStream(blobFromBytes(data), MiB)) {
      streamed.push(c)
    }
    expect(streamed.map((c) => c.length)).toEqual(
      fixed.map((c) => c.data.length),
    )
  })

  it("rejects an invalid chunk size", async () => {
    await expect(async () => {
      for await (const _ of chunkStream(blobFromBytes(makeData(10)), 0)) {
        // unreachable
      }
    }).rejects.toThrowError(BulletinError)
  })
})

describe("planStream", () => {
  it("produces the same root CID and chunk CIDs as prepareStoreChunked", async () => {
    const preparer = new BulletinPreparer({
      defaultChunkSize: MiB,
      createManifest: true,
    })
    const data = makeData(4 * MiB + 4096)

    const prepared = await preparer.prepareStoreChunked(data, {
      chunkSize: MiB,
    })
    const plan = await preparer.planStream(blobFromBytes(data), {
      chunkSize: MiB,
    })

    expect(plan.rootCid?.toString()).toBe(prepared.manifest?.cid.toString())
    expect(plan.chunkCids.map((c) => c.toString())).toEqual(
      prepared.chunks.map((c) => c.cid?.toString()),
    )
    expect(plan.totalSize).toBe(data.length)
    expect(plan.chunkSizes.reduce((a, b) => a + b, 0)).toBe(data.length)
  })

  it("throws on an empty source", async () => {
    const preparer = new BulletinPreparer()
    await expect(
      preparer.planStream(blobFromBytes(new Uint8Array(0))),
    ).rejects.toThrowError(BulletinError)
  })
})

describe("MockBulletinClient streaming", () => {
  it("estimateUpload(source) chunks offline and returns a plan", async () => {
    const client = new MockBulletinClient({
      defaultChunkSize: MiB,
      createManifest: true,
    })
    const data = makeData(3 * MiB + 10)
    const est = await client.estimateUpload(blobFromBytes(data))
    // 4 chunks + 1 manifest.
    expect(est.total).toBe(5)
    expect(est.plan.chunkCids.length).toBe(4)
    expect(est.plan.rootCid).toBeDefined()
    expect(est.transactions).toBe(5) // distinct chunks, nothing skipped
    expect(est.alreadyStored).toEqual([])
  })

  it("submit(estimate, source) resolves to a CID and records a store op", async () => {
    const client = new MockBulletinClient()
    const data = makeData(64 * 1024)
    const src = blobFromBytes(data)
    const { cids } = await client
      .submit(await client.estimateUpload(src), src)
      .send()
    expect(cids[cids.length - 1]!.toString()).toMatch(/^[a-z0-9]+$/i)
    expect(client.getOperations().some((o) => o.type === "store")).toBe(true)
  })

  it("submit(estimate, source) submits the estimate's chunks + manifest", async () => {
    const client = new MockBulletinClient({
      defaultChunkSize: MiB,
      createManifest: true,
    })
    const data = makeData(3 * MiB + 10)
    const src = blobFromBytes(data)
    const est = await client.estimateUpload(src)
    const { cids } = await client.submit(est, src).send()
    // 4 chunks + manifest; last CID is the manifest root.
    expect(cids.length).toBe(5)
    expect(cids[cids.length - 1]!.toString()).toBe(est.plan.rootCid?.toString())
    expect(
      client.getOperations().filter((o) => o.type === "store").length,
    ).toBe(5)
  })

  it("estimateUpload(source) flags an empty source", async () => {
    const client = new MockBulletinClient()
    await expect(
      client.estimateUpload(blobFromBytes(new Uint8Array(0))),
    ).rejects.toThrowError(BulletinError)
    // ErrorCode import kept meaningful — empty data path.
    expect(ErrorCode.EMPTY_DATA).toBeDefined()
  })
})
