// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Bulletin SDK for TypeScript/JavaScript
 *
 * Off-chain client SDK for Polkadot Bulletin Chain that simplifies data storage
 * with automatic chunking, authorization management, and DAG-PB manifest generation.
 *
 * ## Storage Operations (Supported)
 *
 * This SDK provides comprehensive support for storing data on the Bulletin Chain:
 * - CID calculation (content-addressed identifiers)
 * - Data chunking for large files
 * - DAG-PB manifest generation
 * - Transaction preparation and submission
 *
 * ## Data Retrieval (Not Yet Supported)
 *
 * **Important**: This SDK currently does NOT provide data retrieval functionality.
 *
 * ### Deprecated: IPFS Gateway Retrieval
 *
 * Retrieving data via public IPFS gateways (e.g., `https://ipfs.io/ipfs/{cid}`) is
 * **deprecated** and not recommended. Public gateways are centralized infrastructure
 * that goes against the decentralization goals of the Bulletin Chain.
 *
 * ### Future: Smoldot Light Client Retrieval
 *
 * Data retrieval will be supported via the smoldot light client's `bitswap_block` RPC.
 * This approach allows fully decentralized data retrieval directly from Bulletin
 * validator nodes without relying on centralized gateways.
 *
 * See: https://github.com/paritytech/polkadot-bulletin-chain/pull/264
 *
 * ### Current Workaround: Direct P2P via Helia
 *
 * For applications that need retrieval now, connect directly to Bulletin validator
 * nodes using libp2p/Helia with their P2P multiaddrs. This is decentralized but
 * requires additional dependencies. See the console-ui implementation for reference.
 *
 * @packageDocumentation
 */

export { CID } from "multiformats/cid"

// async-client: core client, builders, and public interfaces
export {
  AsyncBulletinClient,
  AuthCallBuilder,
  type AuthCallOptions,
  type BulletinClientInterface,
  type BulletinTypedApi,
  CallBuilder,
  type CallOptions,
  StoreBuilder,
  type SubmitFn,
  type TransactionReceipt,
} from "./async-client.js"

// chunker: data splitting and reassembly
export {
  FixedSizeChunker,
  MAX_CHUNK_SIZE,
  MAX_FILE_SIZE,
  reassembleChunks,
} from "./chunker.js"

// dag: DAG-PB manifest building
export { type DagManifest, UnixFsDagBuilder } from "./dag.js"

// pipeline: high-throughput bulk submission
export {
  pipelineStore,
  type BlockLimits,
  type PipelineConfig,
  type PipelineResult,
  type PipelineStats,
} from "./pipeline.js"

// mock-client: testing support
export {
  MockBulletinClient,
  type MockClientConfig,
  type MockOperation,
} from "./mock-client.js"

// preparer: offline CID calculation and chunking
export { BulletinPreparer } from "./preparer.js"

// types: enums, interfaces, error class, and constants
export {
  AuthorizationScope,
  BulletinError,
  type Chunk,
  type ChunkDetails,
  type ChunkedStoreResult,
  type ChunkerConfig,
  type ChunkProgressEvent,
  ChunkStatus,
  CidCodec,
  type ClientConfig,
  DEFAULT_CHUNKER_CONFIG,
  DEFAULT_CLIENT_CONFIG,
  DEFAULT_STORE_OPTIONS,
  ErrorCode,
  HashAlgorithm,
  type ProgressCallback,
  type ProgressEvent,
  resolveClientConfig,
  type StoreOptions,
  type StoreResult,
  type TransactionStatusEvent,
  TxStatus,
  WaitFor,
} from "./types.js"

// utils: CID utilities, hashing, and authorization estimation
export {
  calculateCid,
  cidFromBytes,
  cidToBytes,
  convertCid,
  estimateAuthorization,
  getContentHash,
  parseCid,
  toBytes,
  validateChunkSize,
} from "./utils.js"
