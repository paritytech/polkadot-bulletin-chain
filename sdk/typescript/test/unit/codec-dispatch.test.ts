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

import { Binary } from "polkadot-api"
import { describe, expect, it } from "vitest"
import { MockBulletinClient } from "../../src/mock-client"
import { isDefaultCidConfig } from "../../src/pipeline"
import { CidCodec, HashAlgorithm } from "../../src/types"

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

describe("Codec dispatch — observable consequence (CIDs differ)", () => {
  it("different codec → different CID for same data", async () => {
    const client = new MockBulletinClient()
    const data = Binary.fromText("identical data")

    const { cids: rawCids } = await client.upload([{ data }]).send()
    const { cids: dagPbCids } = await client
      .upload([{ data, codec: CidCodec.DagPb }])
      .send()
    const { cids: dagCborCids } = await client
      .upload([{ data, codec: CidCodec.DagCbor }])
      .send()

    expect(rawCids[0]!.toString()).not.toBe(dagPbCids[0]!.toString())
    expect(rawCids[0]!.toString()).not.toBe(dagCborCids[0]!.toString())
    expect(dagPbCids[0]!.toString()).not.toBe(dagCborCids[0]!.toString())
  })

  it("different hashAlgo → different CID for same data + codec", async () => {
    const client = new MockBulletinClient()
    const data = Binary.fromText("identical data")

    const { cids: b2 } = await client.upload([{ data }]).send()
    const { cids: sha } = await client
      .upload([{ data, hashAlgo: HashAlgorithm.Sha2_256 }])
      .send()
    const { cids: kec } = await client
      .upload([{ data, hashAlgo: HashAlgorithm.Keccak256 }])
      .send()

    expect(b2[0]!.toString()).not.toBe(sha[0]!.toString())
    expect(b2[0]!.toString()).not.toBe(kec[0]!.toString())
    expect(sha[0]!.toString()).not.toBe(kec[0]!.toString())
  })

  it("identical config → identical CID", async () => {
    const client = new MockBulletinClient()
    const data = Binary.fromText("identical data")

    const { cids: a } = await client.upload([{ data }]).send()
    const { cids: b } = await client.upload([{ data }]).send()
    expect(a[0]!.toString()).toBe(b[0]!.toString())
  })
})
