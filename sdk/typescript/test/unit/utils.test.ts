// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, expect, it } from "vitest"
import { HashAlgorithm } from "../../src/types"
import { getContentHash, validateChunkSize } from "../../src/utils"

describe("Utils", () => {
  describe("validateChunkSize", () => {
    it("should validate valid chunk sizes", () => {
      expect(() => validateChunkSize(1024 * 1024)).not.toThrow() // 1 MiB
      expect(() => validateChunkSize(2 * 1024 * 1024)).not.toThrow() // 2 MiB (MAX_CHUNK_SIZE)
    })

    it("should reject zero size", () => {
      expect(() => validateChunkSize(0)).toThrow()
    })

    it("should reject negative size", () => {
      expect(() => validateChunkSize(-1)).toThrow()
    })

    it("should reject size exceeding maximum", () => {
      expect(() => validateChunkSize(10 * 1024 * 1024)).toThrow()
    })
  })

  describe("getContentHash", () => {
    // Known-answer tests. These pin each hash variant's output to the
    // canonical test vector for the empty string and "abc". They guard
    // against accidental swaps (e.g., `keccak` vs `keccak_256`) and
    // against future implementation changes silently producing different
    // bytes from the chain's hashing.
    const toHex = (b: Uint8Array) =>
      Array.from(b)
        .map((x) => x.toString(16).padStart(2, "0"))
        .join("")
    const empty = new Uint8Array()
    const abc = new TextEncoder().encode("abc")

    it("computes blake2b-256 of empty input", async () => {
      const h = await getContentHash(empty, HashAlgorithm.Blake2b256)
      expect(toHex(h)).toBe(
        "0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8",
      )
    })

    it('computes blake2b-256 of "abc"', async () => {
      const h = await getContentHash(abc, HashAlgorithm.Blake2b256)
      expect(toHex(h)).toBe(
        "bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319",
      )
    })

    it("computes sha2-256 of empty input", async () => {
      const h = await getContentHash(empty, HashAlgorithm.Sha2_256)
      expect(toHex(h)).toBe(
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      )
    })

    it('computes sha2-256 of "abc" (NIST FIPS 180-4 vector)', async () => {
      const h = await getContentHash(abc, HashAlgorithm.Sha2_256)
      expect(toHex(h)).toBe(
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
      )
    })

    it("computes keccak-256 of empty input", async () => {
      const h = await getContentHash(empty, HashAlgorithm.Keccak256)
      expect(toHex(h)).toBe(
        "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
      )
    })

    it('computes keccak-256 of "abc" (Ethereum KAT)', async () => {
      const h = await getContentHash(abc, HashAlgorithm.Keccak256)
      expect(toHex(h)).toBe(
        "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45",
      )
    })

    it("returns 32-byte digest for all algorithms", async () => {
      const data = new Uint8Array([0xde, 0xad, 0xbe, 0xef])
      for (const alg of [
        HashAlgorithm.Blake2b256,
        HashAlgorithm.Sha2_256,
        HashAlgorithm.Keccak256,
      ]) {
        const h = await getContentHash(data, alg)
        expect(h.length).toBe(32)
      }
    })

    it("throws on unsupported hash algorithm", async () => {
      await expect(
        getContentHash(empty, 0xff as HashAlgorithm),
      ).rejects.toThrow()
    })
  })
})
