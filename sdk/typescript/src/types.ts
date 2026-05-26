// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Common types and interfaces for the Bulletin SDK
 */

import type { CID } from "multiformats/cid"
import type { PolkadotSigner } from "polkadot-api"

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
 * One byte payload to upload as a single `store` extrinsic. The SDK computes
 * the CID upfront from `(data, codec, hashAlgo)` and uses it as the item's
 * identifier on every event.
 */
export interface UploadItem {
  data: Uint8Array
  /** CID codec (default Raw = 0x55). DagPb is used for UnixFS manifests. */
  codec?: CidCodec
  /** Multihash algorithm (default Blake2b-256). */
  hashAlgo?: HashAlgorithm
}

/** Final result returned by `upload()`. `cids[i]` matches `items[i]`. */
export interface UploadResult {
  cids: CID[]
}

/**
 * Per-item entry in {@link UploadEstimate.items}. `skipReason` is set when
 * the item won't be submitted (either it duplicates an earlier item in the
 * input, or it's already on chain and the caller opted into `skipExisting`).
 */
export interface UploadEstimateItem {
  index: number
  cid: CID
  bytes: number
  skipReason?: "duplicate_input" | "already_on_chain"
}

/**
 * Result of {@link BulletinClient.estimateUpload}: a per-item dispatch plan
 * plus the aggregated `transactions` / `bytes` the chain will charge to the
 * caller's authorization. Use it to size `authorizeAccount` capacity or to
 * preview an upload in a UI before paying.
 */
export interface UploadEstimate {
  /** Items in the input (= items.length). */
  total: number
  /** Per-item disposition, parallel to the input array. */
  items: UploadEstimateItem[]
  /** Number of `store` extrinsics that would be submitted (= `toUpload.length`). */
  transactions: number
  /** Total bytes consumed by the submitted txs (sum of toUpload items' sizes). */
  bytes: bigint
  /** Indices duplicating an earlier item by content_hash. Always populated. */
  duplicateIndices: number[]
  /** Indices already in the chain's TBCH at estimate time. Populated only if
   * `skipExisting=true`. */
  alreadyStored: number[]
  /** Indices that would be submitted. */
  toUpload: number[]
}

/** Options for {@link BulletinClient.estimateUpload}. */
export interface UploadEstimateOptions {
  /**
   * Query the chain's `TransactionByContentHash` for each unique
   * content_hash and exclude items already present. Requires one RPC per
   * unique content. Default: `false`.
   */
  skipExisting?: boolean
  /**
   * Collapse repeated content_hashes within the input to a single upload
   * (first occurrence wins; subsequent indices land in `duplicateIndices`).
   * Default: `true` — the chain dedupes by content_hash anyway, so charging
   * the caller for the duplicates is wasteful.
   */
  dedupInput?: boolean
}

/**
 * Final result returned by the high-level `uploadFile()`.
 *
 * `cid` is the single identifier the caller uses to retrieve later — the
 * content CID for a single-chunk upload, or the manifest's root CID for a
 * chunked upload.
 */
export interface UploadFileResult {
  cid: CID
}

/**
 * Lifecycle events for an upload. The same set fires whether the upload is
 * a single item or many — single-item uploads just have `total: 1`. Every
 * event carries the item's CID; callers use that to correlate with their
 * own bookkeeping.
 */
export enum UploadStatus {
  ItemStarted = "item_started",
  ItemInBlock = "item_in_block",
  ItemFinalized = "item_finalized",
  ItemFailed = "item_failed",
}

export type UploadEvent =
  | {
      type: UploadStatus.ItemStarted
      index: number
      total: number
      cid: CID
    }
  | {
      type: UploadStatus.ItemInBlock
      index: number
      total: number
      cid: CID
      blockHash: string
      blockNumber: number
      /** Pallet `Stored.index` — the storage slot used by `renew(blockNumber, index)`. */
      extrinsicIndex?: number
    }
  | {
      type: UploadStatus.ItemFinalized
      index: number
      total: number
      cid: CID
      blockHash: string
      blockNumber: number
      /** Pallet `Stored.index` — the storage slot used by `renew(blockNumber, index)`. */
      extrinsicIndex?: number
    }
  | {
      type: UploadStatus.ItemFailed
      index: number
      total: number
      cid: CID
      error: Error
    }

export type UploadCallback = (event: UploadEvent) => void

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
export type ProgressEvent = TransactionStatusEvent

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
  STORE_STALLED = "STORE_STALLED",
  HIJACK_BUDGET_EXCEEDED = "HIJACK_BUDGET_EXCEEDED",
}

/** Error codes that are retryable */
const RETRYABLE_CODES = new Set<ErrorCode>([
  ErrorCode.TRANSACTION_FAILED,
  ErrorCode.TIMEOUT,
  ErrorCode.STORE_STALLED,
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
  [ErrorCode.STORE_STALLED]:
    "Store received no chainHead events from the RPC; the connection may be unhealthy. Retry on a fresh client",
  [ErrorCode.HIJACK_BUDGET_EXCEEDED]:
    "An item's nonce slot was repeatedly hijacked by other transactions from the same signer. Check for concurrent transactions on this account.",
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
 * Per-chain block-capacity constants used by the pipelineStore batch sizer.
 *
 * Determined offline from the chain's runtime constants and pallet benchmarks.
 * Defined here (rather than next to `pipelineStore`) so `ClientConfig` can carry
 * it without a circular import.
 */
export interface BlockLimits {
  /** Max normal-class weight budget (ref_time) per block. */
  maxNormalWeight: bigint
  /** Max normal-class block length in bytes. */
  normalBlockLength: number
  /** Hard per-block limit on store extrinsics (`TransactionStorage::MaxBlockTransactions`). */
  maxBlockTransactions: number
  /** Base weight of a `store` extrinsic (constant part). */
  storeWeightBase: bigint
  /** Per-byte weight slope of a `store` extrinsic. */
  storeWeightPerByte: bigint
  /** Encoding overhead per extrinsic (signature + address + extensions), ~110 bytes. */
  extrinsicOverhead: number
}

/**
 * Reasonable defaults for bulletin-westend / bulletin-paseo runtimes.
 * Derived from runtime constants + pallet benchmarks at the time of writing.
 */
export const DEFAULT_BLOCK_LIMITS: BlockLimits = {
  maxNormalWeight: 1_500_000_000_000n, // 75% of 2s weight budget
  normalBlockLength: 9_437_184, // 90% of 10 MiB MAX_BLOCK_LENGTH
  maxBlockTransactions: 512, // TransactionStorage::MaxBlockTransactions
  storeWeightBase: 35_489_000n,
  storeWeightPerByte: 6_912n,
  extrinsicOverhead: 110,
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
  /** Defensive timeout in milliseconds per transaction (default: 420_000).
   * PAPI handles reconnects and mortality, so this should rarely fire.
   * Set above PAPI's default mortality window (64 blocks ~ 6.4 min at 6s blocks). */
  txTimeout?: number
  /**
   * Factory that returns the JsonRpcProvider instances the SDK should use
   * for the upload pipeline. Called once per `pipelineStore()` invocation
   * — including each outer retry — so dead WS connections from a failed
   * attempt get replaced with fresh ones. `providers[0]` is used for the
   * chainHead monitor; every provider is used as a broadcast target
   * (pass multiple for ws-RPC redundancy across endpoints).
   *
   * - ws-RPC, single node: `() => [getWsProvider(url)]`
   * - ws-RPC, multi-node:  `() => urls.map(getWsProvider)`
   * - smoldot light client: `() => [getSmProvider(chainHandle)]`
   * - custom transport:    `() => [myJsonRpcProvider]`
   *
   * REQUIRED for any signed/unsigned upload — the upload paths throw
   * UNSUPPORTED_OPERATION when this isn't set.
   */
  // biome-ignore lint/suspicious/noExplicitAny: PAPI's JsonRpcProvider type lives in @polkadot-api/json-rpc-provider; avoid a hard import dep here
  providers?: () => any[]
  /** Per-chain block limits used by pipelineStore. Defaults to
   * {@link DEFAULT_BLOCK_LIMITS} (bulletin-westend / paseo values). */
  blockLimits?: BlockLimits
  /**
   * Signer for authorization-class extrinsics (`authorizeAccount`,
   * `authorizePreimage`, `refreshAccountAuthorization`,
   * `refreshPreimageAuthorization`). REQUIRED to call any of those
   * methods — the client will throw `UNSUPPORTED_OPERATION` otherwise.
   * Deliberately separate from the upload signer so an Authorizer key
   * is never used by accident for uploads (and vice-versa).
   */
  authorizerSigner?: PolkadotSigner
  /**
   * Wire-level submission strategy. Today only `"nonce-tracking"` is
   * implemented; the field exists so additional strategies can be added
   * without changing this shape. See `docs/watch-strategy-design.md` for
   * a watch-based design that was prototyped and removed.
   */
  submissionStrategy?: "nonce-tracking"
}

/**
 * Default client configuration values.
 *
 * Used by BulletinClient, MockBulletinClient, and BulletinPreparer
 * so that defaults are defined in one place.
 */
/**
 * Resolved client config — everything from `ClientConfig` is required
 * except `createProvider`, which falls back to the default WebSocket
 * provider in `client.ts` when omitted.
 */
export type ResolvedClientConfig = Required<
  Omit<ClientConfig, "providers" | "authorizerSigner">
> &
  Pick<ClientConfig, "providers" | "authorizerSigner">

export const DEFAULT_CLIENT_CONFIG: ResolvedClientConfig = {
  defaultChunkSize: 1024 * 1024, // 1 MiB
  createManifest: true,
  chunkingThreshold: 2 * 1024 * 1024, // 2 MiB
  txTimeout: 420_000, // 7 minutes (above PAPI's 64-block mortality window)
  blockLimits: DEFAULT_BLOCK_LIMITS,
  submissionStrategy: "nonce-tracking",
  // `providers` (factory) and `authorizerSigner` are intentionally
  // not defaulted — see ResolvedClientConfig.
}

/** Merge caller-supplied config with defaults, ignoring undefined values. */
export function resolveClientConfig(
  config?: Partial<ClientConfig>,
): ResolvedClientConfig {
  if (!config) return { ...DEFAULT_CLIENT_CONFIG }
  const result = { ...DEFAULT_CLIENT_CONFIG }
  for (const key of Object.keys(config) as (keyof ClientConfig)[]) {
    if (config[key] !== undefined) {
      ;(result as Record<string, unknown>)[key] = config[key]
    }
  }
  return result
}
