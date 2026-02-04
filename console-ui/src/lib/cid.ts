import { CID } from "multiformats/cid";
import * as multihash from "multiformats/hashes/digest";

// Hash algorithm codes (multiformats)
export const HASH_CODES = {
  blake2b256: 0xb220,
  sha256: 0x12,
  keccak256: 0x1b,
} as const;

// CID codec codes
export const CID_CODECS = {
  raw: 0x55,
  dagPb: 0x70,
} as const;

/**
 * Hash data using the specified algorithm (browser-compatible)
 */
async function hashData(data: Uint8Array, algorithm: number): Promise<Uint8Array> {
  switch (algorithm) {
    case HASH_CODES.sha256: {
      // Create a new ArrayBuffer copy to ensure compatibility
      const buffer = new ArrayBuffer(data.length);
      new Uint8Array(buffer).set(data);
      const hashBuffer = await crypto.subtle.digest("SHA-256", buffer);
      return new Uint8Array(hashBuffer);
    }
    case HASH_CODES.blake2b256: {
      const { blake2b } = await import("@noble/hashes/blake2b");
      return blake2b(data, { dkLen: 32 });
    }
    case HASH_CODES.keccak256: {
      const { keccak_256 } = await import("@noble/hashes/sha3");
      return keccak_256(data);
    }
    default:
      throw new Error(`Unsupported hash algorithm: 0x${algorithm.toString(16)}`);
  }
}

/**
 * Create a CID from raw bytes
 * Default: raw codec (0x55) with blake2b-256 hash (0xb220)
 */
export async function cidFromBytes(
  data: Uint8Array,
  codec: number = CID_CODECS.raw,
  hashCode: number = HASH_CODES.blake2b256
): Promise<CID> {
  const hash = await hashData(data, hashCode);
  const mh = multihash.create(hashCode, hash);
  return CID.createV1(codec, mh);
}

/**
 * Convert CID to a different codec while preserving the hash
 */
export function convertCidCodec(cid: CID, newCodec: number): CID {
  return CID.createV1(newCodec, cid.multihash);
}

/**
 * Parse and validate a CID string
 */
export function parseCid(cidString: string): CID | null {
  try {
    return CID.parse(cidString.trim());
  } catch {
    return null;
  }
}

/**
 * Get human-readable info about a CID
 */
export function getCidInfo(cid: CID): {
  version: number;
  codec: number;
  codecName: string;
  hashCode: number;
  hashName: string;
  digestSize: number;
} {
  const codecNames: Record<number, string> = {
    0x55: "raw",
    0x70: "dag-pb",
    0x71: "dag-cbor",
  };

  const hashNames: Record<number, string> = {
    0xb220: "blake2b-256",
    0x12: "sha2-256",
    0x1b: "keccak-256",
  };

  return {
    version: cid.version,
    codec: cid.code,
    codecName: codecNames[cid.code] || `unknown (0x${cid.code.toString(16)})`,
    hashCode: cid.multihash.code,
    hashName: hashNames[cid.multihash.code] || `unknown (0x${cid.multihash.code.toString(16)})`,
    digestSize: cid.multihash.size,
  };
}

/**
 * Convert multihash code to runtime HashingAlgorithm enum
 */
export function toHashingEnum(mhCode: number): { type: string } {
  switch (mhCode) {
    case HASH_CODES.blake2b256:
      return { type: "Blake2b256" };
    case HASH_CODES.sha256:
      return { type: "Sha2_256" };
    case HASH_CODES.keccak256:
      return { type: "Keccak256" };
    default:
      throw new Error(`Unhandled multihash code: 0x${mhCode.toString(16)}`);
  }
}

/**
 * Calculate the content hash for data using the specified algorithm
 */
export async function getContentHash(
  data: Uint8Array,
  algorithm: number = HASH_CODES.blake2b256
): Promise<Uint8Array> {
  return hashData(data, algorithm);
}
