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
 * Chunk progress event types
 */
export enum ChunkStatus {
  ChunkStarted = "chunk_started",
  ChunkCompleted = "chunk_completed",
  ChunkFailed = "chunk_failed",
  ManifestStarted = "manifest_started",
  ManifestCreated = "manifest_created",
  Completed = "completed",
}

/**
 * Progress event types for chunked uploads
 */
export type ChunkProgressEvent =
  | { type: ChunkStatus.ChunkStarted; index: number; total: number }
  | { type: ChunkStatus.ChunkCompleted; index: number; total: number; cid: CID }
  | {
      type: ChunkStatus.ChunkFailed
      index: number
      total: number
      error: Error
    }
  | { type: ChunkStatus.ManifestStarted }
  | { type: ChunkStatus.ManifestCreated; cid: CID }
  | { type: ChunkStatus.Completed; manifestCid?: CID }

/**
 * Transaction status event types
 */
export enum TxStatus {
  Signed = "signed",
  Validated = "validated",
  Broadcasted = "broadcasted",
  InBlock = "in_block",
  Finalized = "finalized",
  NoLongerInBlock = "no_longer_in_block",
  Invalid = "invalid",
  Dropped = "dropped",
}

/**
 * Transaction status event types (mirrors PAPI's signSubmitAndWatch events)
 */
export type TransactionStatusEvent =
  | { type: TxStatus.Signed; txHash: string; chunkIndex?: number }
  | { type: TxStatus.Validated; chunkIndex?: number }
  | { type: TxStatus.Broadcasted; chunkIndex?: number }
  | {
      type: TxStatus.InBlock
      blockHash: string
      blockNumber: number
      txIndex?: number
      chunkIndex?: number
    }
  | {
      type: TxStatus.Finalized
      blockHash: string
      blockNumber: number
      txIndex?: number
      chunkIndex?: number
    }
  | { type: TxStatus.NoLongerInBlock; chunkIndex?: number }
  | { type: TxStatus.Invalid; error: string; chunkIndex?: number }
  | { type: TxStatus.Dropped; error: string; chunkIndex?: number }

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
  DATA_TOO_LARGE = "DATA_TOO_LARGE",
  CHUNK_TOO_LARGE = "CHUNK_TOO_LARGE",
  INVALID_CHUNK_SIZE = "INVALID_CHUNK_SIZE",
  INVALID_CONFIG = "INVALID_CONFIG",
  INVALID_CID = "INVALID_CID",
  INVALID_HASH_ALGORITHM = "INVALID_HASH_ALGORITHM",
  CID_CALCULATION_FAILED = "CID_CALCULATION_FAILED",
  DAG_ENCODING_FAILED = "DAG_ENCODING_FAILED",
  INSUFFICIENT_AUTHORIZATION = "INSUFFICIENT_AUTHORIZATION",
  AUTHORIZATION_FAILED = "AUTHORIZATION_FAILED",
  TRANSACTION_FAILED = "TRANSACTION_FAILED",
  CHUNK_FAILED = "CHUNK_FAILED",
  MISSING_CHUNK = "MISSING_CHUNK",
  TIMEOUT = "TIMEOUT",
  UNSUPPORTED_OPERATION = "UNSUPPORTED_OPERATION",
}

/** Error codes that are retryable */
const RETRYABLE_CODES = new Set<ErrorCode>([
  ErrorCode.TRANSACTION_FAILED,
  ErrorCode.TIMEOUT,
])

/** Recovery hints per error code */
const RECOVERY_HINTS: Record<ErrorCode, string> = {
  [ErrorCode.EMPTY_DATA]: "Provide non-empty data",
  [ErrorCode.DATA_TOO_LARGE]: "Reduce data size or use chunked upload",
  [ErrorCode.CHUNK_TOO_LARGE]: "Reduce chunk size to 2 MiB or less",
  [ErrorCode.INVALID_CHUNK_SIZE]: "Use a chunk size between 1 byte and 2 MiB",
  [ErrorCode.INVALID_CONFIG]: "Check configuration parameters",
  [ErrorCode.INVALID_CID]: "Verify CID format",
  [ErrorCode.INVALID_HASH_ALGORITHM]:
    "Use blake2b-256, sha2-256, or keccak-256",
  [ErrorCode.CID_CALCULATION_FAILED]: "Verify data and hash algorithm",
  [ErrorCode.DAG_ENCODING_FAILED]: "Check chunk CIDs and data integrity",
  [ErrorCode.INSUFFICIENT_AUTHORIZATION]:
    "Request additional authorization via authorizeAccount()",
  [ErrorCode.AUTHORIZATION_FAILED]:
    "Check that the account has authorizer privileges",
  [ErrorCode.TRANSACTION_FAILED]:
    "Verify transaction parameters and account nonce",
  [ErrorCode.CHUNK_FAILED]: "Verify data integrity and chunker configuration",
  [ErrorCode.MISSING_CHUNK]:
    "Ensure all chunks are present with contiguous indices starting from 0",
  [ErrorCode.TIMEOUT]:
    "Transaction was not finalized within the timeout window. Retry the transaction",
  [ErrorCode.UNSUPPORTED_OPERATION]:
    "This operation is not supported in this context",
}

/**
 * SDK error class
 */
export class BulletinError extends Error {
  constructor(
    message: string,
    public readonly code: ErrorCode,
    override readonly cause?: unknown,
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
  /** Timeout in milliseconds per transaction (default: 420_000).
   * Set above PAPI's default mortality window (64 blocks ~ 6.4 min at 6s blocks)
   * so it never races against PAPI's own InvalidTxError. Only fires if PAPI
   * itself never reports back (e.g. connection drop). */
  txTimeout?: number
}

/**
 * Default client configuration values.
 *
 * Used by AsyncBulletinClient, MockBulletinClient, and BulletinPreparer
 * so that defaults are defined in one place.
 */
export const DEFAULT_CLIENT_CONFIG: Required<ClientConfig> = {
  defaultChunkSize: 1024 * 1024, // 1 MiB
  createManifest: true,
  chunkingThreshold: 2 * 1024 * 1024, // 2 MiB
  txTimeout: 420_000, // 7 minutes (above PAPI's 64-block mortality window)
}
