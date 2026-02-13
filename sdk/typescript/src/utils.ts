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
 * Convert hex string to Uint8Array
 */
export function hexToBytes(hex: string): Uint8Array {
  const cleanHex = hex.startsWith("0x") ? hex.slice(2) : hex;
  const bytes = new Uint8Array(cleanHex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(cleanHex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

/**
 * Convert Uint8Array to hex string
 */
export function bytesToHex(bytes: Uint8Array): string {
  return (
    "0x" +
    Array.from(bytes)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("")
  );
}

/**
 * Format bytes as human-readable size
 *
 * @example
 * ```typescript
 * formatBytes(1024); // '1.00 KB'
 * formatBytes(1048576); // '1.00 MB'
 * ```
 */
export function formatBytes(bytes: number, decimals: number = 2): string {
  if (bytes === 0) return "0 Bytes";

  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ["Bytes", "KB", "MB", "GB", "TB"];

  const i = Math.floor(Math.log(bytes) / Math.log(k));

  return `${(bytes / Math.pow(k, i)).toFixed(dm)} ${sizes[i]}`;
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
      `Chunk size ${formatBytes(size)} exceeds maximum ${formatBytes(MAX_CHUNK_SIZE)}`,
      "CHUNK_TOO_LARGE",
    );
  }
}

/**
 * Calculate optimal chunk size for given data size
 *
 * Returns a chunk size that balances transaction overhead and throughput.
 *
 * @example
 * ```typescript
 * const size = optimalChunkSize(100_000_000); // 100 MB
 * // Returns 1048576 (1 MiB)
 * ```
 */
export function optimalChunkSize(dataSize: number): number {
  const MIN_CHUNK_SIZE = 1024 * 1024; // 1 MiB
  const OPTIMAL_CHUNKS = 100; // Target chunk count

  if (dataSize <= MIN_CHUNK_SIZE) {
    return dataSize;
  }

  const optimalSize = Math.floor(dataSize / OPTIMAL_CHUNKS);

  if (optimalSize < MIN_CHUNK_SIZE) {
    return MIN_CHUNK_SIZE;
  } else if (optimalSize > MAX_CHUNK_SIZE) {
    return MAX_CHUNK_SIZE;
  } else {
    // Round to nearest MiB
    return Math.floor(optimalSize / 1_048_576) * 1_048_576;
  }
}

/**
 * Estimate transaction fees for given data size
 *
 * This is a rough estimate and actual fees may vary.
 *
 * @example
 * ```typescript
 * const fees = estimateFees(1_000_000); // 1 MB
 * ```
 */
export function estimateFees(dataSize: number): bigint {
  // Base fee + per-byte fee
  // These are placeholder values - actual fees depend on chain configuration
  const BASE_FEE = 1_000_000n; // Base transaction fee
  const PER_BYTE_FEE = 100n; // Fee per byte

  return BASE_FEE + BigInt(dataSize) * PER_BYTE_FEE;
}

/**
 * Retry helper for async operations
 *
 * @example
 * ```typescript
 * const result = await retry(
 *   async () => await someOperation(),
 *   { maxRetries: 3, delayMs: 1000 }
 * );
 * ```
 */
export async function retry<T>(
  fn: () => Promise<T>,
  options: {
    maxRetries?: number;
    delayMs?: number;
    exponentialBackoff?: boolean;
  } = {},
): Promise<T> {
  const { maxRetries = 3, delayMs = 1000, exponentialBackoff = true } = options;

  let lastError: Error | undefined;

  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    try {
      return await fn();
    } catch (error) {
      lastError = error as Error;

      if (attempt < maxRetries) {
        const delay = exponentialBackoff
          ? delayMs * Math.pow(2, attempt)
          : delayMs;

        await sleep(delay);
      }
    }
  }

  throw lastError || new Error("Retry failed");
}

/**
 * Sleep for specified milliseconds
 *
 * @example
 * ```typescript
 * await sleep(1000); // Wait 1 second
 * ```
 */
export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Batch an array into chunks of specified size
 *
 * @example
 * ```typescript
 * const items = [1, 2, 3, 4, 5];
 * const batches = batch(items, 2);
 * // [[1, 2], [3, 4], [5]]
 * ```
 */
export function batch<T>(array: T[], size: number): T[][] {
  const batches: T[][] = [];

  for (let i = 0; i < array.length; i += size) {
    batches.push(array.slice(i, i + size));
  }

  return batches;
}

/**
 * Run promises with concurrency limit
 *
 * @example
 * ```typescript
 * const urls = ['url1', 'url2', 'url3', ...];
 * const results = await limitConcurrency(
 *   urls.map(url => () => fetch(url)),
 *   5 // Max 5 concurrent requests
 * );
 * ```
 */
export async function limitConcurrency<T>(
  tasks: (() => Promise<T>)[],
  limit: number,
): Promise<T[]> {
  const results: T[] = [];
  const executing: Promise<void>[] = [];

  for (const task of tasks) {
    const promise = task().then((result) => {
      results.push(result);
    });

    executing.push(promise);

    if (executing.length >= limit) {
      await Promise.race(executing);
      const index = await Promise.race(
        executing.map((p, i) => p.then(() => i)),
      );
      executing.splice(index, 1);
    }
  }

  await Promise.all(executing);

  return results;
}

/**
 * Create a progress tracker
 *
 * @example
 * ```typescript
 * const tracker = createProgressTracker(100);
 *
 * tracker.increment(); // Progress: 1%
 * tracker.increment(9); // Progress: 10%
 * ```
 */
export function createProgressTracker(total: number) {
  let current = 0;

  return {
    get current() {
      return current;
    },
    get total() {
      return total;
    },
    get percentage() {
      return total > 0 ? (current / total) * 100 : 0;
    },
    increment(amount: number = 1) {
      current = Math.min(current + amount, total);
      return this.percentage;
    },
    set(value: number) {
      current = Math.max(0, Math.min(value, total));
      return this.percentage;
    },
    reset() {
      current = 0;
    },
    isComplete() {
      return current >= total;
    },
  };
}

/**
 * Measure execution time of an async function
 *
 * @example
 * ```typescript
 * const [result, duration] = await measureTime(async () => {
 *   return await someOperation();
 * });
 *
 * console.log(`Operation took ${duration}ms`);
 * ```
 */
export async function measureTime<T>(
  fn: () => Promise<T>,
): Promise<[T, number]> {
  const start = Date.now();
  const result = await fn();
  const duration = Date.now() - start;

  return [result, duration];
}

/**
 * Calculate throughput (bytes per second)
 *
 * @example
 * ```typescript
 * const mbps = calculateThroughput(1_048_576, 1000); // 1 MB in 1 second
 * // Returns 1048576 (bytes/s) = 1 MB/s
 * ```
 */
export function calculateThroughput(bytes: number, ms: number): number {
  if (ms === 0) return 0;
  return (bytes / ms) * 1000; // bytes per second
}

/**
 * Format throughput as human-readable string
 *
 * @example
 * ```typescript
 * formatThroughput(1048576); // '1.00 MB/s'
 * ```
 */
export function formatThroughput(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

/**
 * Validate SS58 address format
 *
 * Basic validation - checks format only, not checksum
 *
 * @example
 * ```typescript
 * isValidSS58('5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY'); // true
 * ```
 */
export function isValidSS58(address: string): boolean {
  // SS58 addresses typically start with 1-5 and are base58 encoded
  // This is a basic check - full validation requires the ss58-registry
  const ss58Regex = /^[1-9A-HJ-NP-Za-km-z]{47,48}$/;
  return ss58Regex.test(address);
}

/**
 * Truncate string with ellipsis
 *
 * @example
 * ```typescript
 * truncate('bafkreiabcd1234567890', 15); // 'bafkr...67890'
 * ```
 */
export function truncate(
  str: string,
  maxLength: number,
  ellipsis: string = "...",
): string {
  if (str.length <= maxLength) {
    return str;
  }

  const partLength = Math.floor((maxLength - ellipsis.length) / 2);
  // Correctly handle odd/even splits so the total length equals maxLength
  const front = str.slice(0, Math.ceil((maxLength - ellipsis.length) / 2));
  const back = str.slice(-Math.floor((maxLength - ellipsis.length) / 2));

  return front + ellipsis + back;
}

/**
 * Deep clone an object (JSON-serializable objects only)
 *
 * @example
 * ```typescript
 * const copy = deepClone(original);
 * ```
 */
export function deepClone<T>(obj: T): T {
  return JSON.parse(JSON.stringify(obj));
}

/**
 * Check if code is running in Node.js environment
 */
export function isNode(): boolean {
  return (
    typeof process !== "undefined" &&
    process.versions != null &&
    process.versions.node != null
  );
}

/**
 * Check if code is running in browser environment
 */
export function isBrowser(): boolean {
  return (
    typeof window !== "undefined" && typeof window.document !== "undefined"
  );
}
