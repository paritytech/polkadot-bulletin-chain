// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, expect, it } from "vitest"
import { UnixFsDagBuilder } from "../../src/dag"
import { BulletinError, ErrorCode, HashAlgorithm } from "../../src/types"
import { calculateCid } from "../../src/utils"

describe("UnixFsDagBuilder", () => {
  const builder = new UnixFsDagBuilder()

  describe("build", () => {
    it("should throw EMPTY_DATA for empty chunks array", async () => {
      await expect(builder.build([])).rejects.toThrow(BulletinError)
      await expect(builder.build([])).rejects.toMatchObject({
        code: ErrorCode.EMPTY_DATA,
      })
    })

    it("should throw DAG_ENCODING_FAILED for chunk without CID", async () => {
      const chunks = [
        { data: new Uint8Array([1, 2, 3]), index: 0, totalChunks: 1 },
      ]
      await expect(builder.build(chunks)).rejects.toThrow(BulletinError)
      await expect(builder.build(chunks)).rejects.toMatchObject({
        code: ErrorCode.DAG_ENCODING_FAILED,
      })
    })

    it("should build a valid DAG manifest from chunks with CIDs", async () => {
      const data1 = new Uint8Array([1, 2, 3])
      const data2 = new Uint8Array([4, 5, 6])
      const cid1 = await calculateCid(data1, 0x55, HashAlgorithm.Blake2b256)
      const cid2 = await calculateCid(data2, 0x55, HashAlgorithm.Blake2b256)

      const chunks = [
        { data: data1, cid: cid1, index: 0, totalChunks: 2 },
        { data: data2, cid: cid2, index: 1, totalChunks: 2 },
      ]

      const manifest = await builder.build(chunks)

      expect(manifest.rootCid).toBeDefined()
      expect(manifest.chunkCids).toHaveLength(2)
      expect(manifest.totalSize).toBe(6)
      expect(manifest.dagBytes).toBeInstanceOf(Uint8Array)
      expect(manifest.dagBytes.length).toBeGreaterThan(0)
    })
  })

  describe("parse", () => {
    it("should round-trip a built manifest", async () => {
      const data1 = new Uint8Array([10, 20, 30])
      const data2 = new Uint8Array([40, 50])
      const cid1 = await calculateCid(data1, 0x55, HashAlgorithm.Blake2b256)
      const cid2 = await calculateCid(data2, 0x55, HashAlgorithm.Blake2b256)

      const chunks = [
        { data: data1, cid: cid1, index: 0, totalChunks: 2 },
        { data: data2, cid: cid2, index: 1, totalChunks: 2 },
      ]

      const manifest = await builder.build(chunks)
      const parsed = await builder.parse(manifest.dagBytes)

      expect(parsed.chunkCids).toHaveLength(2)
      expect(parsed.chunkCids[0].toString()).toBe(cid1.toString())
      expect(parsed.chunkCids[1].toString()).toBe(cid2.toString())
      expect(parsed.totalSize).toBe(5) // 3 + 2
    })

    it("should throw DAG_ENCODING_FAILED for invalid bytes", async () => {
      const invalidBytes = new Uint8Array([0xff, 0xfe, 0xfd])

      await expect(builder.parse(invalidBytes)).rejects.toThrow(BulletinError)
      await expect(builder.parse(invalidBytes)).rejects.toMatchObject({
        code: ErrorCode.DAG_ENCODING_FAILED,
      })
    })

    it("should throw DAG_ENCODING_FAILED for empty bytes", async () => {
      await expect(builder.parse(new Uint8Array(0))).rejects.toThrow(
        BulletinError,
      )
      await expect(builder.parse(new Uint8Array(0))).rejects.toMatchObject({
        code: ErrorCode.DAG_ENCODING_FAILED,
      })
    })

    it("should include 'Failed to decode' in error message", async () => {
      try {
        await builder.parse(new Uint8Array([0x00]))
        expect.fail("Should have thrown")
      } catch (error) {
        expect(error).toBeInstanceOf(BulletinError)
        expect((error as BulletinError).message).toContain(
          "Failed to decode DAG-PB manifest",
        )
      }
    })
  })
})
