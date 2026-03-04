// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Common types and interfaces for the Bulletin SDK
 */

import type { CID } from "multiformats/cid"

/**
 * CID codec types supported by Bulletin Chain.
 *
 * For custom codecs not listed here, pass the numeric multicodec code directly
 * wherever a `CidCodec | number` is accepted.
 */
export enum CidCodec {
  /** Raw binary (0x55) */
  Raw = 0x55,
  /** DAG-PB (0x70) */
  DagPb = 0x70,
  /** DAG-CBOR (0x71) */
  DagCbor = 0x71,
}

/**
 * Hash algorithm types supported by Bulletin Chain
 */
export enum HashAlgorithm {
  /** BLAKE2b-256 (0xb220) */
  Blake2b256 = 0xb220,
  /** SHA2-256 (0x12) */
  Sha2_256 = 0x12,
  /** Keccak-256 (0x1b) */
  Keccak256 = 0x1b,
}

/**
 * Configuration for chunking large data
 */
export interface ChunkerConfig {
  /** Size of each chunk in bytes (default: 1 MiB) */
  chunkSize: number
  /** Whether to create a DAG-PB manifest (default: true) */
  createManifest: boolean
}

/**
 * Default chunker configuration
 *
 * Uses 1 MiB chunk size by default (safe and efficient for most use cases).
 * Maximum allowed is 2 MiB (MAX_CHUNK_SIZE, Bitswap limit for IPFS compatibility).
 */
export const DEFAULT_CHUNKER_CONFIG: ChunkerConfig = {
  chunkSize: 1024 * 1024, // 1 MiB (default)
  createManifest: true,
}

/**
 * A single chunk of data
 */
export interface Chunk {
  /** The chunk data */
  data: Uint8Array
  /** The CID of this chunk (calculated after encoding) */
  cid?: CID
  /** Index of this chunk in the sequence */
  index: number
  /** Total number of chunks */
  totalChunks: number
}

/**
 * Transaction confirmation level
 *
 * Can be used as a value (`WaitFor.InBlock`) or as a type (`WaitFor`).
 */
export type WaitFor = "in_block" | "finalized"
export const WaitFor = {
  InBlock: "in_block" as const,
  Finalized: "finalized" as const,
}

/**
 * Options for storing data
 */
export interface StoreOptions {
  /** CID codec to use (default: raw). Accepts a `CidCodec` or a custom numeric multicodec code. */
  cidCodec?: CidCodec | number
  /** Hashing algorithm to use (default: blake2b-256) */
  hashingAlgorithm?: HashAlgorithm
  /**
   * What to wait for before returning (default: "in_block")
   * - "in_block": Return when tx is in a best block (faster, may reorg)
   * - "finalized": Return when tx is finalized (safer, slower)
   */
  waitFor?: WaitFor
}

/**
 * Default store options
 */
export const DEFAULT_STORE_OPTIONS: StoreOptions = {
  cidCodec: CidCodec.Raw,
  hashingAlgorithm: HashAlgorithm.Blake2b256,
  waitFor: "in_block",
}

/**
 * Details about chunks in a chunked upload
 */
export interface ChunkDetails {
  /** CIDs of all stored chunks */
  chunkCids: CID[]
  /** Number of chunks */
  numChunks: number
}

/**
 * Result of a storage operation
 *
 * This result type works for both single-transaction uploads and chunked uploads.
 * For chunked uploads, the `cid` field contains the manifest CID, and `chunks`
 * contains details about the individual chunks.
 *
 * When chunked without a manifest (`withManifest(false)`), `cid` is undefined
 * and the individual chunk CIDs are in `chunks.chunkCids`.
 */
export interface StoreResult {
  /** The primary CID of the stored data
   * - For single uploads: CID of the data
   * - For chunked uploads with manifest: CID of the manifest
   * - For chunked uploads without manifest: undefined
   */
  cid?: CID
  /** Size of the stored data in bytes */
  size: number
  /** Block number where data was stored (if known) */
  blockNumber?: number
  /** Extrinsic index within the block (required for renew operations)
   * This value comes from the `Stored` event's `index` field
   */
  extrinsicIndex?: number
  /** Chunk details (only present for chunked uploads) */
  chunks?: ChunkDetails
}

/**
 * Result of a chunked storage operation
 */
export interface ChunkedStoreResult {
  /** CIDs of all stored chunks */
  chunkCids: CID[]
  /** The manifest CID (if manifest was created) */
  manifestCid?: CID
  /** Total size of all chunks in bytes */
  totalSize: number
  /** Number of chunks */
  numChunks: number
}

/**
 * Authorization scope types (mirrors the pallet's AuthorizationScope enum)
 */
export enum AuthorizationScope {
  /** Account-based authorization */
  Account = "Account",
  /** Preimage-based authorization (content-addressed) */
  Preimage = "Preimage",
}

/**
 * Progress event types for chunked uploads
 */
export type ChunkProgressEvent =
  | { type: "chunk_started"; index: number; total: number }
  | { type: "chunk_completed"; index: number; total: number; cid: CID }
  | { type: "chunk_failed"; index: number; total: number; error: Error }
  | { type: "manifest_started" }
  | { type: "manifest_created"; cid: CID }
  | { type: "completed"; manifestCid?: CID }

/**
 * Transaction status event types (mirrors PAPI's signSubmitAndWatch events)
 */
export type TransactionStatusEvent =
  | { type: "signed"; txHash: string; chunkIndex?: number }
  | { type: "validated" }
  | { type: "broadcasted"; numPeers?: number; chunkIndex?: number }
  | { type: "in_best_block"; blockHash: string; blockNumber: number; txIndex?: number; chunkIndex?: number }
  | { type: "finalized"; blockHash: string; blockNumber: number; txIndex?: number; chunkIndex?: number }
  | { type: "no_longer_in_best_block" }
  | { type: "invalid"; error: string }
  | { type: "dropped"; error: string }

/**
 * Combined progress event types
 */
export type ProgressEvent = ChunkProgressEvent | TransactionStatusEvent

/**
 * Progress callback type
 */
export type ProgressCallback = (event: ProgressEvent) => void

/**
 * Error codes for the Bulletin SDK.
 *
 * These codes are consistent with the Rust SDK's `Error::code()` method.
 */
export enum ErrorCode {
  EMPTY_DATA = "EMPTY_DATA",
  FILE_TOO_LARGE = "FILE_TOO_LARGE",
  CHUNK_TOO_LARGE = "CHUNK_TOO_LARGE",
  INVALID_CHUNK_SIZE = "INVALID_CHUNK_SIZE",
  INVALID_CONFIG = "INVALID_CONFIG",
  INVALID_CID = "INVALID_CID",
  UNSUPPORTED_HASH_ALGORITHM = "UNSUPPORTED_HASH_ALGORITHM",
  INVALID_HASH_ALGORITHM = "INVALID_HASH_ALGORITHM",
  CID_CALCULATION_FAILED = "CID_CALCULATION_FAILED",
  DAG_ENCODING_FAILED = "DAG_ENCODING_FAILED",
  DAG_DECODING_FAILED = "DAG_DECODING_FAILED",
  AUTHORIZATION_NOT_FOUND = "AUTHORIZATION_NOT_FOUND",
  INSUFFICIENT_AUTHORIZATION = "INSUFFICIENT_AUTHORIZATION",
  AUTHORIZATION_EXPIRED = "AUTHORIZATION_EXPIRED",
  AUTHORIZATION_FAILED = "AUTHORIZATION_FAILED",
  SUBMISSION_FAILED = "SUBMISSION_FAILED",
  TRANSACTION_FAILED = "TRANSACTION_FAILED",
  STORAGE_FAILED = "STORAGE_FAILED",
  NETWORK_ERROR = "NETWORK_ERROR",
  CHUNKING_FAILED = "CHUNKING_FAILED",
  CHUNK_FAILED = "CHUNK_FAILED",
  RETRIEVAL_FAILED = "RETRIEVAL_FAILED",
  RENEWAL_NOT_FOUND = "RENEWAL_NOT_FOUND",
  RENEWAL_FAILED = "RENEWAL_FAILED",
  TIMEOUT = "TIMEOUT",
  UNSUPPORTED_OPERATION = "UNSUPPORTED_OPERATION",
  RETRY_EXHAUSTED = "RETRY_EXHAUSTED",
}

/** Error codes that are retryable */
const RETRYABLE_CODES = new Set<string>([
  ErrorCode.AUTHORIZATION_EXPIRED,
  ErrorCode.NETWORK_ERROR,
  ErrorCode.STORAGE_FAILED,
  ErrorCode.SUBMISSION_FAILED,
  ErrorCode.TRANSACTION_FAILED,
  ErrorCode.RETRIEVAL_FAILED,
  ErrorCode.RENEWAL_FAILED,
  ErrorCode.TIMEOUT,
])

/** Recovery hints per error code */
const RECOVERY_HINTS: Record<string, string> = {
  [ErrorCode.EMPTY_DATA]: "Provide non-empty data",
  [ErrorCode.FILE_TOO_LARGE]: "Reduce file size or use chunked upload",
  [ErrorCode.CHUNK_TOO_LARGE]: "Reduce chunk size to 8 MiB or less",
  [ErrorCode.INVALID_CHUNK_SIZE]: "Use a chunk size between 1 byte and 8 MiB",
  [ErrorCode.INVALID_CONFIG]: "Check configuration parameters",
  [ErrorCode.INVALID_CID]: "Verify CID format",
  [ErrorCode.UNSUPPORTED_HASH_ALGORITHM]:
    "Use blake2b-256, sha2-256, or keccak-256",
  [ErrorCode.INVALID_HASH_ALGORITHM]:
    "Use blake2b-256, sha2-256, or keccak-256",
  [ErrorCode.CID_CALCULATION_FAILED]: "Verify data and hash algorithm",
  [ErrorCode.DAG_ENCODING_FAILED]: "Check chunk CIDs and data integrity",
  [ErrorCode.DAG_DECODING_FAILED]: "Verify DAG-PB data format",
  [ErrorCode.AUTHORIZATION_NOT_FOUND]:
    "Call authorizeAccount() or authorizePreimage() first",
  [ErrorCode.INSUFFICIENT_AUTHORIZATION]: "Request additional authorization",
  [ErrorCode.AUTHORIZATION_EXPIRED]:
    "Call refreshAccountAuthorization() to extend expiry",
  [ErrorCode.AUTHORIZATION_FAILED]:
    "Check that the account has authorizer privileges",
  [ErrorCode.SUBMISSION_FAILED]: "Check node connectivity and try again",
  [ErrorCode.TRANSACTION_FAILED]:
    "Verify transaction parameters and account nonce",
  [ErrorCode.STORAGE_FAILED]: "Check node connectivity and try again",
  [ErrorCode.NETWORK_ERROR]: "Check network connectivity to the RPC endpoint",
  [ErrorCode.CHUNKING_FAILED]:
    "Verify data integrity and chunker configuration",
  [ErrorCode.CHUNK_FAILED]: "Verify data integrity and chunker configuration",
  [ErrorCode.RETRIEVAL_FAILED]: "The data may not be available yet; try again",
  [ErrorCode.RENEWAL_NOT_FOUND]: "Verify the block number and extrinsic index",
  [ErrorCode.RENEWAL_FAILED]: "Check that storage hasn't expired, then retry",
  [ErrorCode.TIMEOUT]: "Increase timeout or retry",
  [ErrorCode.UNSUPPORTED_OPERATION]:
    "This operation is not supported in this context",
  [ErrorCode.RETRY_EXHAUSTED]:
    "All retry attempts failed; check underlying cause",
}

/**
 * SDK error class
 */
export class BulletinError extends Error {
  constructor(
    message: string,
    public readonly code: ErrorCode | string,
    public readonly cause?: unknown,
  ) {
    super(message, { cause })
    this.name = "BulletinError"
  }

  /** Whether this error is likely transient and retrying may succeed. */
  get retryable(): boolean {
    return RETRYABLE_CODES.has(this.code)
  }

  /** An actionable recovery suggestion for this error. */
  get recoveryHint(): string {
    return RECOVERY_HINTS[this.code] ?? "No recovery hint available"
  }
}

/**
 * Client configuration
 */
export interface ClientConfig {
  /** Default chunk size for large files (default: 1 MiB) */
  defaultChunkSize?: number
  /** Whether to create manifests for chunked uploads (default: true) */
  createManifest?: boolean
  /** Threshold for automatic chunking (default: 2 MiB).
   * Data larger than this will be automatically chunked by `store()`. */
  chunkingThreshold?: number
}
