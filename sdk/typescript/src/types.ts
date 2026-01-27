// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Common types and interfaces for the Bulletin SDK
 */

import { CID } from 'multiformats/cid';

/**
 * CID codec types supported by Bulletin Chain
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
  chunkSize: number;
  /** Maximum number of parallel uploads (default: 8) */
  maxParallel: number;
  /** Whether to create a DAG-PB manifest (default: true) */
  createManifest: boolean;
}

/**
 * Default chunker configuration
 */
export const DEFAULT_CHUNKER_CONFIG: ChunkerConfig = {
  chunkSize: 1024 * 1024, // 1 MiB
  maxParallel: 8,
  createManifest: true,
};

/**
 * A single chunk of data
 */
export interface Chunk {
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
export interface StoreOptions {
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
export const DEFAULT_STORE_OPTIONS: StoreOptions = {
  cidCodec: CidCodec.Raw,
  hashingAlgorithm: HashAlgorithm.Blake2b256,
  waitForFinalization: false,
};

/**
 * Result of a storage operation
 */
export interface StoreResult {
  /** The CID of the stored data */
  cid: CID;
  /** Size of the stored data in bytes */
  size: number;
  /** Block number where data was stored (if known) */
  blockNumber?: number;
}

/**
 * Result of a chunked storage operation
 */
export interface ChunkedStoreResult {
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
export enum AuthorizationScope {
  /** Account-based authorization (more flexible) */
  Account = 'Account',
  /** Preimage-based authorization (content-addressed) */
  Preimage = 'Preimage',
}

/**
 * Authorization information
 */
export interface Authorization {
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
export type ProgressEvent =
  | { type: 'chunk_started'; index: number; total: number }
  | { type: 'chunk_completed'; index: number; total: number; cid: CID }
  | { type: 'chunk_failed'; index: number; total: number; error: Error }
  | { type: 'manifest_started' }
  | { type: 'manifest_created'; cid: CID }
  | { type: 'completed'; manifestCid?: CID };

/**
 * Progress callback type
 */
export type ProgressCallback = (event: ProgressEvent) => void;

/**
 * SDK error class
 */
export class BulletinError extends Error {
  constructor(
    message: string,
    public readonly code: string,
    public readonly cause?: unknown,
  ) {
    super(message);
    this.name = 'BulletinError';
  }
}

/**
 * Client configuration
 */
export interface ClientConfig {
  /** RPC endpoint URL */
  endpoint: string;
  /** Default chunk size for large files (default: 1 MiB) */
  defaultChunkSize?: number;
  /** Maximum parallel uploads (default: 8) */
  maxParallel?: number;
  /** Whether to create manifests for chunked uploads (default: true) */
  createManifest?: boolean;
}
