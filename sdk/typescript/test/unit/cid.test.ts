// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, it, expect } from 'vitest';
import { calculateCid, CidCodec, HashAlgorithm } from '../../src/cid';
import { CID } from 'multiformats/cid';

describe('CID Calculation', () => {
  const testData = new TextEncoder().encode('Hello, Bulletin Chain!');

  it('should calculate CID with default options (Raw, Blake2b-256)', async () => {
    const cid = await calculateCid(testData);

    expect(cid).toBeInstanceOf(CID);
    expect(cid.code).toBe(CidCodec.Raw);
    expect(cid.multihash.code).toBe(HashAlgorithm.Blake2b256);
  });

  it('should calculate CID with Raw codec', async () => {
    const cid = await calculateCid(testData, CidCodec.Raw, HashAlgorithm.Blake2b256);

    expect(cid).toBeInstanceOf(CID);
    expect(cid.code).toBe(CidCodec.Raw);
    expect(cid.toString()).toMatch(/^[a-z0-9]+$/i);
  });

  it('should calculate CID with DagPb codec', async () => {
    const cid = await calculateCid(testData, CidCodec.DagPb, HashAlgorithm.Sha2_256);

    expect(cid).toBeInstanceOf(CID);
    expect(cid.code).toBe(CidCodec.DagPb);
    expect(cid.multihash.code).toBe(HashAlgorithm.Sha2_256);
  });

  it('should calculate CID with SHA2-256 hash', async () => {
    const cid = await calculateCid(testData, CidCodec.Raw, HashAlgorithm.Sha2_256);

    expect(cid).toBeInstanceOf(CID);
    expect(cid.multihash.code).toBe(HashAlgorithm.Sha2_256);
  });

  it('should calculate different CIDs for different data', async () => {
    const data1 = new TextEncoder().encode('Data 1');
    const data2 = new TextEncoder().encode('Data 2');

    const cid1 = await calculateCid(data1);
    const cid2 = await calculateCid(data2);

    expect(cid1.toString()).not.toBe(cid2.toString());
  });

  it('should calculate same CID for same data', async () => {
    const cid1 = await calculateCid(testData);
    const cid2 = await calculateCid(testData);

    expect(cid1.toString()).toBe(cid2.toString());
  });

  it('should calculate different CIDs for different hash algorithms', async () => {
    const cidBlake2 = await calculateCid(testData, CidCodec.Raw, HashAlgorithm.Blake2b256);
    const cidSha2 = await calculateCid(testData, CidCodec.Raw, HashAlgorithm.Sha2_256);

    expect(cidBlake2.toString()).not.toBe(cidSha2.toString());
  });

  it('should handle empty data', async () => {
    const emptyData = new Uint8Array(0);
    const cid = await calculateCid(emptyData);

    expect(cid).toBeInstanceOf(CID);
    expect(cid.toString()).toMatch(/^[a-z0-9]+$/i);
  });

  it('should handle large data', async () => {
    const largeData = new Uint8Array(10 * 1024 * 1024).fill(0x42); // 10 MiB
    const cid = await calculateCid(largeData);

    expect(cid).toBeInstanceOf(CID);
    expect(cid.toString()).toMatch(/^[a-z0-9]+$/i);
  });

  it('should be deterministic across multiple calls', async () => {
    const cids = await Promise.all([
      calculateCid(testData),
      calculateCid(testData),
      calculateCid(testData),
    ]);

    const cidStrings = cids.map(c => c.toString());
    expect(cidStrings[0]).toBe(cidStrings[1]);
    expect(cidStrings[1]).toBe(cidStrings[2]);
  });
});
