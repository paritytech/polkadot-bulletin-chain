import { CID } from 'multiformats/cid';
export { CID } from 'multiformats/cid';
import { PolkadotSigner } from 'polkadot-api';

/**
 * Common types and interfaces for the Bulletin SDK
 */

/**
 * CID codec types supported by Bulletin Chain
 */
declare enum CidCodec {
    /** Raw binary (0x55) */
    Raw = 85,
    /** DAG-PB (0x70) */
    DagPb = 112,
    /** DAG-CBOR (0x71) */
    DagCbor = 113
}
/**
 * Hash algorithm types supported by Bulletin Chain
 */
declare enum HashAlgorithm {
    /** BLAKE2b-256 (0xb220) */
    Blake2b256 = 45600,
    /** SHA2-256 (0x12) */
    Sha2_256 = 18,
    /** Keccak-256 (0x1b) */
    Keccak256 = 27
}
/**
 * Configuration for chunking large data
 */
interface ChunkerConfig {
    /** Size of each chunk in bytes (default: 1 MiB) */
    chunkSize: number;
    /** Maximum number of parallel uploads (default: 8) */
    maxParallel: number;
    /** Whether to create a DAG-PB manifest (default: true) */
    createManifest: boolean;
}
/**
 * Default chunker configuration
 */
declare const DEFAULT_CHUNKER_CONFIG: ChunkerConfig;
/**
 * A single chunk of data
 */
interface Chunk {
    /** The chunk data */
    data: Uint8Array;
    /** The CID of this chunk (calculated after encoding) */
    cid?: CID;
    /** Index of this chunk in the sequence */
    index: number;
    /** Total number of chunks */
    totalChunks: number;
}
/**
 * Options for storing data
 */
interface StoreOptions {
    /** CID codec to use (default: raw) */
    cidCodec?: CidCodec;
    /** Hashing algorithm to use (default: blake2b-256) */
    hashingAlgorithm?: HashAlgorithm;
    /** Whether to wait for finalization (default: false) */
    waitForFinalization?: boolean;
}
/**
 * Default store options
 */
declare const DEFAULT_STORE_OPTIONS: StoreOptions;
/**
 * Details about chunks in a chunked upload
 */
interface ChunkDetails {
    /** CIDs of all stored chunks */
    chunkCids: CID[];
    /** Number of chunks */
    numChunks: number;
}
/**
 * Result of a storage operation
 *
 * This result type works for both single-transaction uploads and chunked uploads.
 * For chunked uploads, the `cid` field contains the manifest CID, and `chunks`
 * contains details about the individual chunks.
 */
interface StoreResult {
    /** The primary CID of the stored data
     * - For single uploads: CID of the data
     * - For chunked uploads: CID of the manifest
     */
    cid: CID;
    /** Size of the stored data in bytes */
    size: number;
    /** Block number where data was stored (if known) */
    blockNumber?: number;
    /** Chunk details (only present for chunked uploads) */
    chunks?: ChunkDetails;
}
/**
 * Result of a chunked storage operation
 */
interface ChunkedStoreResult {
    /** CIDs of all stored chunks */
    chunkCids: CID[];
    /** The manifest CID (if manifest was created) */
    manifestCid?: CID;
    /** Total size of all chunks in bytes */
    totalSize: number;
    /** Number of chunks */
    numChunks: number;
}
/**
 * Authorization scope types
 */
declare enum AuthorizationScope {
    /** Account-based authorization (more flexible) */
    Account = "Account",
    /** Preimage-based authorization (content-addressed) */
    Preimage = "Preimage"
}
/**
 * Authorization information
 */
interface Authorization {
    /** The authorization scope */
    scope: AuthorizationScope;
    /** Number of transactions authorized */
    transactions: number;
    /** Maximum total size in bytes */
    maxSize: bigint;
    /** Block number when authorization expires (if known) */
    expiresAt?: number;
}
/**
 * Progress event types
 */
type ProgressEvent = {
    type: 'chunk_started';
    index: number;
    total: number;
} | {
    type: 'chunk_completed';
    index: number;
    total: number;
    cid: CID;
} | {
    type: 'chunk_failed';
    index: number;
    total: number;
    error: Error;
} | {
    type: 'manifest_started';
} | {
    type: 'manifest_created';
    cid: CID;
} | {
    type: 'completed';
    manifestCid?: CID;
};
/**
 * Progress callback type
 */
type ProgressCallback = (event: ProgressEvent) => void;
/**
 * SDK error class
 */
declare class BulletinError extends Error {
    readonly code: string;
    readonly cause?: unknown | undefined;
    constructor(message: string, code: string, cause?: unknown | undefined);
}
/**
 * Client configuration
 */
interface ClientConfig {
    /** RPC endpoint URL */
    endpoint: string;
    /** Default chunk size for large files (default: 1 MiB) */
    defaultChunkSize?: number;
    /** Maximum parallel uploads (default: 8) */
    maxParallel?: number;
    /** Whether to create manifests for chunked uploads (default: true) */
    createManifest?: boolean;
    /** Threshold for automatic chunking (default: 2 MiB).
     * Data larger than this will be automatically chunked by `store()`. */
    chunkingThreshold?: number;
    /** Check authorization before uploading to fail fast (default: true).
     * Queries blockchain for current authorization and validates before submission. */
    checkAuthorizationBeforeUpload?: boolean;
}

/**
 * Data chunking utilities for splitting large files into smaller pieces
 */

/** Maximum chunk size allowed (2 MiB, matches Bitswap limit) */
declare const MAX_CHUNK_SIZE: number;
/**
 * Fixed-size chunker that splits data into equal-sized chunks
 */
declare class FixedSizeChunker {
    private config;
    constructor(config?: Partial<ChunkerConfig>);
    /**
     * Split data into chunks
     */
    chunk(data: Uint8Array): Chunk[];
    /**
     * Calculate the number of chunks needed for the given data size
     */
    numChunks(dataSize: number): number;
    /**
     * Get the chunk size
     */
    get chunkSize(): number;
}
/**
 * Reassemble chunks back into the original data
 */
declare function reassembleChunks(chunks: Chunk[]): Uint8Array;

/**
 * DAG-PB (Directed Acyclic Graph - Protocol Buffers) utilities
 * for creating IPFS-compatible manifests
 */

/**
 * DAG-PB manifest representing a file composed of multiple chunks
 */
interface DagManifest {
    /** The root CID of the manifest */
    rootCid: CID;
    /** CIDs of all chunks in order */
    chunkCids: CID[];
    /** Total size of the file in bytes */
    totalSize: number;
    /** Encoded DAG-PB bytes */
    dagBytes: Uint8Array;
}
/**
 * UnixFS DAG-PB builder following IPFS UnixFS v1 specification
 */
declare class UnixFsDagBuilder {
    /**
     * Build a UnixFS DAG-PB file node from raw chunks
     */
    build(chunks: Chunk[], hashAlgorithm?: HashAlgorithm): Promise<DagManifest>;
    /**
     * Parse a DAG-PB manifest back into its components
     */
    parse(dagBytes: Uint8Array): Promise<{
        chunkCids: CID[];
        totalSize: number;
    }>;
}

/**
 * Utility functions for CID calculation and data manipulation
 */

/**
 * Calculate content hash using the specified algorithm
 *
 * Note: For production use, integrate with the pallet's hashing functions
 * via PAPI to ensure exact compatibility.
 */
declare function getContentHash(data: Uint8Array, hashAlgorithm: HashAlgorithm): Promise<Uint8Array>;
/**
 * Create a CID for data with specified codec and hashing algorithm
 *
 * Default to raw codec (0x55) with blake2b-256 hash (0xb220)
 */
declare function calculateCid(data: Uint8Array, cidCodec?: number, hashAlgorithm?: HashAlgorithm): Promise<CID>;
/**
 * Convert CID to different codec while keeping the same hash
 */
declare function convertCid(cid: CID, newCodec: number): CID;
/**
 * Parse CID from string
 */
declare function parseCid(cidString: string): CID;
/**
 * Parse CID from bytes
 */
declare function cidFromBytes(bytes: Uint8Array): CID;
/**
 * Convert CID to bytes
 */
declare function cidToBytes(cid: CID): Uint8Array;
/**
 * Convert hex string to Uint8Array
 */
declare function hexToBytes(hex: string): Uint8Array;
/**
 * Convert Uint8Array to hex string
 */
declare function bytesToHex(bytes: Uint8Array): string;
/**
 * Format bytes as human-readable size
 *
 * @example
 * ```typescript
 * formatBytes(1024); // '1.00 KB'
 * formatBytes(1048576); // '1.00 MB'
 * ```
 */
declare function formatBytes(bytes: number, decimals?: number): string;
/**
 * Validate chunk size
 *
 * @throws BulletinError if chunk size is invalid
 */
declare function validateChunkSize(size: number): void;
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
declare function optimalChunkSize(dataSize: number): number;
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
declare function estimateFees(dataSize: number): bigint;
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
declare function retry<T>(fn: () => Promise<T>, options?: {
    maxRetries?: number;
    delayMs?: number;
    exponentialBackoff?: boolean;
}): Promise<T>;
/**
 * Sleep for specified milliseconds
 *
 * @example
 * ```typescript
 * await sleep(1000); // Wait 1 second
 * ```
 */
declare function sleep(ms: number): Promise<void>;
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
declare function batch<T>(array: T[], size: number): T[][];
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
declare function limitConcurrency<T>(tasks: (() => Promise<T>)[], limit: number): Promise<T[]>;
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
declare function createProgressTracker(total: number): {
    readonly current: number;
    readonly total: number;
    readonly percentage: number;
    increment(amount?: number): number;
    set(value: number): number;
    reset(): void;
    isComplete(): boolean;
};
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
declare function measureTime<T>(fn: () => Promise<T>): Promise<[T, number]>;
/**
 * Calculate throughput (bytes per second)
 *
 * @example
 * ```typescript
 * const mbps = calculateThroughput(1_048_576, 1000); // 1 MB in 1 second
 * // Returns 1048576 (bytes/s) = 1 MB/s
 * ```
 */
declare function calculateThroughput(bytes: number, ms: number): number;
/**
 * Format throughput as human-readable string
 *
 * @example
 * ```typescript
 * formatThroughput(1048576); // '1.00 MB/s'
 * ```
 */
declare function formatThroughput(bytesPerSecond: number): string;
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
declare function isValidSS58(address: string): boolean;
/**
 * Truncate string with ellipsis
 *
 * @example
 * ```typescript
 * truncate('bafkreiabcd1234567890', 15); // 'bafkr...67890'
 * ```
 */
declare function truncate(str: string, maxLength: number, ellipsis?: string): string;
/**
 * Deep clone an object (JSON-serializable objects only)
 *
 * @example
 * ```typescript
 * const copy = deepClone(original);
 * ```
 */
declare function deepClone<T>(obj: T): T;
/**
 * Check if code is running in Node.js environment
 */
declare function isNode(): boolean;
/**
 * Check if code is running in browser environment
 */
declare function isBrowser(): boolean;

/**
 * High-level client for interacting with Bulletin Chain
 */

/**
 * High-level client for Bulletin Chain operations
 *
 * This provides a simplified API for common operations like storing
 * and retrieving data, with automatic chunking and manifest creation.
 *
 * For full blockchain integration, use PAPI (@polkadot-api) to submit
 * transactions to the TransactionStorage pallet.
 */
declare class BulletinClient {
    private config;
    constructor(config: ClientConfig);
    /**
     * Prepare a simple store operation (data < 2 MiB)
     *
     * Returns the data and its CID. Use PAPI to submit to TransactionStorage.store
     */
    prepareStore(data: Uint8Array, options?: StoreOptions): Promise<{
        data: Uint8Array;
        cid: CID;
    }>;
    /**
     * Prepare a chunked store operation for large files
     *
     * This chunks the data, calculates CIDs, and optionally creates a DAG-PB manifest.
     * Returns chunk data and manifest that can be submitted via PAPI.
     */
    prepareStoreChunked(data: Uint8Array, config?: Partial<ChunkerConfig>, options?: StoreOptions, progressCallback?: ProgressCallback): Promise<{
        chunks: Chunk[];
        manifest?: {
            data: Uint8Array;
            cid: CID;
        };
    }>;
    /**
     * Estimate authorization needed for storing data
     *
     * Returns (num_transactions, total_bytes) needed for authorization
     */
    estimateAuthorization(dataSize: number): {
        transactions: number;
        bytes: number;
    };
}

/**
 * Transaction submission for Bulletin Chain operations
 */

/**
 * Transaction receipt from a successful submission
 */
interface TransactionReceipt {
    /** Block hash containing the transaction */
    blockHash: string;
    /** Transaction hash */
    txHash: string;
    /** Block number (if known) */
    blockNumber?: number;
}
/**
 * Transaction submitter interface
 *
 * Implement this to integrate with your signing and submission method
 */
interface TransactionSubmitter {
    /** Submit a store transaction */
    submitStore(data: Uint8Array): Promise<TransactionReceipt>;
    /** Submit an authorize_account transaction */
    submitAuthorizeAccount(who: string, transactions: number, bytes: bigint): Promise<TransactionReceipt>;
    /** Submit an authorize_preimage transaction */
    submitAuthorizePreimage(contentHash: Uint8Array, maxSize: bigint): Promise<TransactionReceipt>;
    /** Submit a renew transaction */
    submitRenew(block: number, index: number): Promise<TransactionReceipt>;
    /** Submit a refresh_account_authorization transaction */
    submitRefreshAccountAuthorization(who: string): Promise<TransactionReceipt>;
    /** Submit a refresh_preimage_authorization transaction */
    submitRefreshPreimageAuthorization(contentHash: Uint8Array): Promise<TransactionReceipt>;
    /** Submit a remove_expired_account_authorization transaction */
    submitRemoveExpiredAccountAuthorization(who: string): Promise<TransactionReceipt>;
    /** Submit a remove_expired_preimage_authorization transaction */
    submitRemoveExpiredPreimageAuthorization(contentHash: Uint8Array): Promise<TransactionReceipt>;
    /**
     * Query authorization state for an account
     *
     * Returns undefined if this submitter doesn't support queries or if no authorization exists.
     */
    queryAccountAuthorization?(who: string): Promise<Authorization | undefined>;
    /**
     * Query authorization state for a preimage
     *
     * Returns undefined if this submitter doesn't support queries or if no authorization exists.
     */
    queryPreimageAuthorization?(contentHash: Uint8Array): Promise<Authorization | undefined>;
    /**
     * Query the current block number
     *
     * Returns undefined if this submitter doesn't support queries.
     */
    queryCurrentBlock?(): Promise<number | undefined>;
}
/**
 * PAPI-based transaction submitter
 *
 * Complete implementation using Polkadot API (PAPI)
 *
 * Note: Query methods (queryAccountAuthorization, queryPreimageAuthorization, queryCurrentBlock)
 * are not implemented by default. To enable authorization pre-flight checking, extend this class
 * and implement the query methods to query the blockchain state.
 */
declare class PAPITransactionSubmitter implements TransactionSubmitter {
    private api;
    private signer;
    constructor(api: any, signer: PolkadotSigner);
    submitStore(data: Uint8Array): Promise<TransactionReceipt>;
    submitAuthorizeAccount(who: string, transactions: number, bytes: bigint): Promise<TransactionReceipt>;
    submitAuthorizePreimage(contentHash: Uint8Array, maxSize: bigint): Promise<TransactionReceipt>;
    submitRenew(block: number, index: number): Promise<TransactionReceipt>;
    submitRefreshAccountAuthorization(who: string): Promise<TransactionReceipt>;
    submitRefreshPreimageAuthorization(contentHash: Uint8Array): Promise<TransactionReceipt>;
    submitRemoveExpiredAccountAuthorization(who: string): Promise<TransactionReceipt>;
    submitRemoveExpiredPreimageAuthorization(contentHash: Uint8Array): Promise<TransactionReceipt>;
}

/**
 * Async Bulletin client that submits transactions to the chain
 *
 * This client provides a complete interface for storing data on Bulletin Chain,
 * handling everything from chunking to transaction submission.
 */
declare class AsyncBulletinClient {
    private submitter;
    private config;
    private account?;
    constructor(submitter: TransactionSubmitter, config?: Partial<ClientConfig>);
    /**
     * Set the account for authorization checks
     *
     * If set and `checkAuthorizationBeforeUpload` is enabled, the client will
     * query authorization state before uploading and fail fast if insufficient.
     */
    withAccount(account: string): this;
    /**
     * Store data on Bulletin Chain
     *
     * Automatically chunks data if it exceeds the configured threshold.
     * This handles the complete workflow:
     * 1. Decide whether to chunk based on data size
     * 2. Calculate CID(s)
     * 3. Submit transaction(s)
     * 4. Wait for finalization
     *
     * @param data - Data to store
     * @param options - Storage options (CID codec, hash algorithm)
     * @param progressCallback - Optional callback for progress tracking (only called for chunked uploads)
     */
    store(data: Uint8Array, options?: StoreOptions, progressCallback?: ProgressCallback): Promise<StoreResult>;
    /**
     * Internal: Store data in a single transaction (no chunking)
     */
    private storeInternalSingle;
    /**
     * Calculate authorization requirements for chunked upload
     */
    private calculateRequirements;
    /**
     * Internal: Store data with chunking (returns unified StoreResult)
     */
    private storeInternalChunked;
    /**
     * Store large data with automatic chunking and manifest creation
     *
     * Handles the complete workflow:
     * 1. Chunk the data
     * 2. Calculate CIDs for each chunk
     * 3. Submit each chunk as a separate transaction
     * 4. Create and submit DAG-PB manifest (if enabled)
     * 5. Return all CIDs and receipt information
     */
    storeChunked(data: Uint8Array, config?: Partial<ChunkerConfig>, options?: StoreOptions, progressCallback?: ProgressCallback): Promise<ChunkedStoreResult>;
    /**
     * Authorize an account to store data
     *
     * Requires sudo/authorizer privileges
     */
    authorizeAccount(who: string, transactions: number, bytes: bigint): Promise<TransactionReceipt>;
    /**
     * Authorize a preimage (by content hash) to be stored
     *
     * Requires sudo/authorizer privileges
     */
    authorizePreimage(contentHash: Uint8Array, maxSize: bigint): Promise<TransactionReceipt>;
    /**
     * Renew/extend retention period for stored data
     */
    renew(block: number, index: number): Promise<TransactionReceipt>;
    /**
     * Refresh an account authorization (extends expiry)
     *
     * Requires sudo/authorizer privileges
     */
    refreshAccountAuthorization(who: string): Promise<TransactionReceipt>;
    /**
     * Refresh a preimage authorization (extends expiry)
     *
     * Requires sudo/authorizer privileges
     */
    refreshPreimageAuthorization(contentHash: Uint8Array): Promise<TransactionReceipt>;
    /**
     * Remove an expired account authorization
     */
    removeExpiredAccountAuthorization(who: string): Promise<TransactionReceipt>;
    /**
     * Remove an expired preimage authorization
     */
    removeExpiredPreimageAuthorization(contentHash: Uint8Array): Promise<TransactionReceipt>;
    /**
     * Estimate authorization needed for storing data
     */
    estimateAuthorization(dataSize: number): {
        transactions: number;
        bytes: number;
    };
}

/**
 * Bulletin SDK for TypeScript/JavaScript
 *
 * Off-chain client SDK for Polkadot Bulletin Chain that simplifies data storage
 * with automatic chunking, authorization management, and DAG-PB manifest generation.
 *
 * @packageDocumentation
 */

/**
 * SDK version
 */
declare const VERSION = "0.1.0";

export { AsyncBulletinClient, type Authorization, AuthorizationScope, BulletinClient, BulletinError, type Chunk, type ChunkDetails, type ChunkedStoreResult, type ChunkerConfig, CidCodec, type ClientConfig, DEFAULT_CHUNKER_CONFIG, DEFAULT_STORE_OPTIONS, type DagManifest, FixedSizeChunker, HashAlgorithm, MAX_CHUNK_SIZE, PAPITransactionSubmitter, type ProgressCallback, type ProgressEvent, type StoreOptions, type StoreResult, type TransactionReceipt, type TransactionSubmitter, UnixFsDagBuilder, VERSION, batch, bytesToHex, calculateCid, calculateThroughput, cidFromBytes, cidToBytes, convertCid, createProgressTracker, deepClone, estimateFees, formatBytes, formatThroughput, getContentHash, hexToBytes, isBrowser, isNode, isValidSS58, limitConcurrency, measureTime, optimalChunkSize, parseCid, reassembleChunks, retry, sleep, truncate, validateChunkSize };
