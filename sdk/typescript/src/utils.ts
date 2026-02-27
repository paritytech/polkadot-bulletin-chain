// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Utility functions for CID calculation and data manipulation
 */

import { CID } from "multiformats/cid";
import * as digest from "multiformats/hashes/digest";
import { blake2AsU8a, sha256AsU8a } from "@polkadot/util-crypto";
import { HashAlgorithm, BulletinError } from "./types.js";
import { MAX_CHUNK_SIZE } from "./chunker.js";

/**
 * Calculate content hash using the specified algorithm
 *
 * Note: For production use, integrate with the pallet's hashing functions
 * via PAPI to ensure exact compatibility.
 */
export async function getContentHash(
  data: Uint8Array,
  hashAlgorithm: HashAlgorithm,
): Promise<Uint8Array> {
  switch (hashAlgorithm) {
    case HashAlgorithm.Blake2b256: {
      return blake2AsU8a(data);
    }
    case HashAlgorithm.Sha2_256: {
      return sha256AsU8a(data);
    }
    case HashAlgorithm.Keccak256:
      // Note: Keccak256 requires additional library
      // Users should integrate with pallet's hashing via PAPI
      throw new BulletinError(
        "Keccak256 hashing requires integration with the pallet via PAPI",
        "UNSUPPORTED_HASH_ALGORITHM",
      );
    default:
      throw new BulletinError(
        `Unsupported hash algorithm: ${hashAlgorithm}`,
        "INVALID_HASH_ALGORITHM",
      );
  }
}

/**
 * Create a CID for data with specified codec and hashing algorithm
 *
 * Default to raw codec (0x55) with blake2b-256 hash (0xb220)
 */
export async function calculateCid(
  data: Uint8Array,
  cidCodec: number = 0x55,
  hashAlgorithm: HashAlgorithm = HashAlgorithm.Blake2b256,
): Promise<CID> {
  try {
    // Calculate content hash
    const hash = await getContentHash(data, hashAlgorithm);

    // Create multihash digest
    const mh = digest.create(hashAlgorithm, hash);

    // Create CIDv1
    return CID.createV1(cidCodec, mh);
  } catch (error) {
    throw new BulletinError(
      `Failed to calculate CID: ${error}`,
      "CID_CALCULATION_FAILED",
      error,
    );
  }
}

/**
 * Convert CID to different codec while keeping the same hash
 */
export function convertCid(cid: CID, newCodec: number): CID {
  return CID.createV1(newCodec, cid.multihash);
}

/**
 * Parse CID from string
 */
export function parseCid(cidString: string): CID {
  try {
    return CID.parse(cidString);
  } catch (error) {
    throw new BulletinError(
      `Failed to parse CID: ${error}`,
      "INVALID_CID",
      error,
    );
  }
}

/**
 * Parse CID from bytes
 */
export function cidFromBytes(bytes: Uint8Array): CID {
  try {
    return CID.decode(bytes);
  } catch (error) {
    throw new BulletinError(
      `Failed to decode CID from bytes: ${error}`,
      "INVALID_CID",
      error,
    );
  }
}

/**
 * Convert CID to bytes
 */
export function cidToBytes(cid: CID): Uint8Array {
  return cid.bytes;
}

/**
 * Validate chunk size
 *
 * @throws BulletinError if chunk size is invalid
 */
export function validateChunkSize(size: number): void {
  if (size <= 0) {
    throw new BulletinError(
      "Chunk size must be positive",
      "INVALID_CHUNK_SIZE",
    );
  }

  if (size > MAX_CHUNK_SIZE) {
    throw new BulletinError(
      `Chunk size ${size} bytes exceeds maximum ${MAX_CHUNK_SIZE} bytes`,
      "CHUNK_TOO_LARGE",
    );
  }
}

