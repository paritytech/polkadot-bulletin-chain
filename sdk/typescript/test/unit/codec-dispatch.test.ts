// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Tests for the per-item codec dispatch decision in `pipelineStore.signBatch`:
 * default codec (Raw + Blake2b-256) → `TransactionStorage.store({data})`;
 * anything else → `TransactionStorage.store_with_cid_config({cid, data})`.
 *
 * We don't unit-test the actual extrinsic encoding (PAPI does that), only
 * the SDK's `isDefaultCidConfig` decision + the observable consequence
 * (different codec → different on-chain CID).
 */

import { describe, expect, it } from "vitest"
import { blobFromItems } from "../../src/blob-source"
import { MockBulletinClient } from "../../src/mock-client"
import { isDefaultCidConfig } from "../../src/pipeline"
import { CidCodec, HashAlgorithm, type UploadItem } from "../../src/types"

describe("Codec dispatch — isDefaultCidConfig truth table", () => {
  const data = new Uint8Array([1, 2, 3])

  it("default (no codec, no hashAlgo) → true", () => {
    expect(isDefaultCidConfig({ data })).toBe(true)
  })

  it("explicit Raw + Blake2b256 → true", () => {
    expect(
      isDefaultCidConfig({
        data,
        codec: CidCodec.Raw,
        hashAlgo: HashAlgorithm.Blake2b256,
      }),
    ).toBe(true)
  })

  it("DagPb codec → false", () => {
    expect(isDefaultCidConfig({ data, codec: CidCodec.DagPb })).toBe(false)
  })

  it("DagCbor codec → false", () => {
    expect(isDefaultCidConfig({ data, codec: CidCodec.DagCbor })).toBe(false)
  })

  it("non-default hash → false", () => {
    expect(isDefaultCidConfig({ data, hashAlgo: HashAlgorithm.Sha2_256 })).toBe(
      false,
    )
  })

  it("non-default both → false", () => {
    expect(
      isDefaultCidConfig({
        data,
        codec: CidCodec.DagPb,
        hashAlgo: HashAlgorithm.Keccak256,
      }),
    ).toBe(false)
  })
})

// Submit items via the sole submission API (items-as-is plan + items source).
async function submitItems(client: MockBulletinClient, items: UploadItem[]) {
  const src = blobFromItems(items)
  return client.submit(await client.estimateUpload(items), src).send()
}

describe("Codec dispatch — observable consequence (CIDs differ)", () => {
  it("different codec → different CID for same data", async () => {
    const client = new MockBulletinClient()
    const data = new TextEncoder().encode("identical data")

    const { cids: rawCids } = await submitItems(client, [{ data }])
    const { cids: dagPbCids } = await submitItems(client, [
      { data, codec: CidCodec.DagPb },
    ])
    const { cids: dagCborCids } = await submitItems(client, [
      { data, codec: CidCodec.DagCbor },
    ])

    expect(rawCids[0]!.toString()).not.toBe(dagPbCids[0]!.toString())
    expect(rawCids[0]!.toString()).not.toBe(dagCborCids[0]!.toString())
    expect(dagPbCids[0]!.toString()).not.toBe(dagCborCids[0]!.toString())
  })

  it("different hashAlgo → different CID for same data + codec", async () => {
    const client = new MockBulletinClient()
    const data = new TextEncoder().encode("identical data")

    const { cids: b2 } = await submitItems(client, [{ data }])
    const { cids: sha } = await submitItems(client, [
      { data, hashAlgo: HashAlgorithm.Sha2_256 },
    ])
    const { cids: kec } = await submitItems(client, [
      { data, hashAlgo: HashAlgorithm.Keccak256 },
    ])

    expect(b2[0]!.toString()).not.toBe(sha[0]!.toString())
    expect(b2[0]!.toString()).not.toBe(kec[0]!.toString())
    expect(sha[0]!.toString()).not.toBe(kec[0]!.toString())
  })

  it("identical config → identical CID", async () => {
    const client = new MockBulletinClient()
    const data = new TextEncoder().encode("identical data")

    const { cids: a } = await submitItems(client, [{ data }])
    const { cids: b } = await submitItems(client, [{ data }])
    expect(a[0]!.toString()).toBe(b[0]!.toString())
  })
})
