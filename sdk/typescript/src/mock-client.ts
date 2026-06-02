// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Mock client for testing without a blockchain connection
 *
 * This module provides a mock implementation of the Bulletin client that
 * doesn't require a running node. It's useful for:
 * - Unit testing application logic
 * - Integration tests without node setup
 * - Development and prototyping
 */

import type { CID } from "multiformats/cid"
import type { BlobSource, SeekableSource } from "./blob-source.js"
import {
  AuthCallBuilder,
  type AuthorizeAccountEntry,
  type BulletinClientInterface,
  CallBuilder,
  SubmitBuilder,
  type TransactionReceipt,
} from "./client.js"
import { BulletinPreparer } from "./preparer.js"
import {
  BulletinError,
  type ChunkPlan,
  CidCodec,
  type ClientConfig,
  ErrorCode,
  HashAlgorithm,
  type ResolvedClientConfig,
  resolveClientConfig,
  type StreamEstimate,
  type UploadEstimate,
  type UploadEstimateItem,
  type UploadEstimateOptions,
  type UploadItem,
  type UploadResult,
  UploadStatus,
} from "./types.js"
import { calculateCid, cidToContentHashHex } from "./utils.js"

/**
 * Configuration for the mock Bulletin client
 */
export interface MockClientConfig extends ClientConfig {
  /** Simulate authorization failures (for testing error paths) */
  simulateAuthFailure?: boolean
  /** Simulate storage failures (for testing error paths) */
  simulateStorageFailure?: boolean
  /** Simulate insufficient authorization (for testing pre-check error path) */
  simulateInsufficientAuth?: boolean
}

/**
 * Record of a mock operation performed
 */
export type MockOperation =
  | { type: "store"; dataSize: number; cid: string }
  | {
      type: "authorize_account"
      who: string
      transactions: number
      bytes: bigint
    }
  | { type: "authorize_preimage"; contentHash: Uint8Array; maxSize: bigint }
  | { type: "refresh_account_authorization"; who: string }
  | {
      type: "refresh_preimage_authorization"
      contentHash: Uint8Array
    }
  | { type: "renew"; block: number; index: number }
  | { type: "store_preimage_auth"; dataSize: number; cid: string }
  | { type: "remove_expired_account_authorization"; who: string }
  | {
      type: "remove_expired_preimage_authorization"
      contentHash: Uint8Array
    }

const MOCK_BLOCK_HASH =
  "0x0000000000000000000000000000000000000000000000000000000000000001"
const MOCK_TX_HASH =
  "0x0000000000000000000000000000000000000000000000000000000000000002"

function mockReceipt(): TransactionReceipt {
  return { blockHash: MOCK_BLOCK_HASH, txHash: MOCK_TX_HASH, blockNumber: 1 }
}

/**
 * Mock Bulletin client for testing
 *
 * This client simulates blockchain operations without requiring a running node.
 * It calculates CIDs correctly and tracks operations but doesn't actually submit
 * transactions to a chain.
 *
 * @example
 * ```typescript
 * import { MockBulletinClient, blobFromItems } from '@parity/bulletin-sdk';
 *
 * const client = new MockBulletinClient();
 * const items = [{ data }];
 * const { cids } = await client
 *   .submit(await client.estimateUpload(items), blobFromItems(items))
 *   .send();
 * console.log('Mock CID:', cids[0].toString());
 *
 * // Inspect the recorded operations.
 * expect(client.getOperations()).toHaveLength(1);
 * ```
 */
export class MockBulletinClient implements BulletinClientInterface {
  /** Client configuration */
  public config: ResolvedClientConfig & {
    simulateAuthFailure: boolean
    simulateStorageFailure: boolean
    simulateInsufficientAuth: boolean
  }
  /** Operations performed (for testing verification) */
  private operations: MockOperation[] = []
  /** Offline preparer for streaming estimates (chunking + CID, no chain). */
  private preparer: BulletinPreparer

  /**
   * Create a new mock client with optional configuration
   */
  constructor(config?: Partial<MockClientConfig>) {
    this.config = {
      ...resolveClientConfig(config),
      simulateAuthFailure: config?.simulateAuthFailure ?? false,
      simulateStorageFailure: config?.simulateStorageFailure ?? false,
      simulateInsufficientAuth: config?.simulateInsufficientAuth ?? false,
    }
    this.preparer = new BulletinPreparer({
      defaultChunkSize: this.config.defaultChunkSize,
      createManifest: this.config.createManifest,
      chunkingThreshold: this.config.chunkingThreshold,
    })
  }

  /**
   * Get all operations performed by this client
   */
  getOperations(): MockOperation[] {
    return [...this.operations]
  }

  /**
   * Clear recorded operations
   */
  clearOperations(): void {
    this.operations = []
  }

  /**
   * No-op for the mock client — present to satisfy `BulletinClientInterface`.
   */
  async destroy(): Promise<void> {}

  submit(estimate: StreamEstimate, _source: SeekableSource): SubmitBuilder {
    return new SubmitBuilder(async (_waitFor, onEvent, checkAuth) => {
      if (checkAuth && this.config.simulateInsufficientAuth) {
        throw new BulletinError(
          "Account is not authorized to store data on this chain",
          ErrorCode.INSUFFICIENT_AUTHORIZATION,
        )
      }
      if (this.config.simulateStorageFailure) {
        throw new BulletinError(
          "Simulated storage failure",
          ErrorCode.TRANSACTION_FAILED,
        )
      }
      const plan = estimate.plan
      const cids: CID[] = [...plan.chunkCids]
      const sizes: number[] = [...plan.chunkSizes]
      if (plan.rootCid && plan.manifestData) {
        cids.push(plan.rootCid)
        sizes.push(plan.manifestData.length)
      }
      const total = cids.length
      const skip = new Set(estimate.alreadyStored)
      for (let i = 0; i < cids.length; i++) {
        if (skip.has(i)) continue
        const cid = cids[i] as CID
        this.operations.push({
          type: "store",
          dataSize: sizes[i] ?? 0,
          cid: cid.toString(),
        })
        onEvent?.({ type: UploadStatus.ItemStarted, index: i, total, cid })
        onEvent?.({
          type: UploadStatus.ItemFinalized,
          index: i,
          total,
          cid,
          blockHash: MOCK_BLOCK_HASH,
          blockNumber: 1,
        })
      }
      return { cids }
    })
  }

  private throwIfAuthFailure(): void {
    if (this.config.simulateAuthFailure) {
      throw new BulletinError(
        "Simulated authorization failure",
        ErrorCode.AUTHORIZATION_FAILED,
      )
    }
  }

  authorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
  ): AuthCallBuilder
  authorizeAccount(entries: AuthorizeAccountEntry[]): AuthCallBuilder
  authorizeAccount(
    whoOrEntries: string | AuthorizeAccountEntry[],
    transactions?: number,
    bytes?: bigint,
  ): AuthCallBuilder {
    return new AuthCallBuilder(async () => {
      this.throwIfAuthFailure()
      const entries: AuthorizeAccountEntry[] =
        typeof whoOrEntries === "string"
          ? [{ who: whoOrEntries, transactions: transactions!, bytes: bytes! }]
          : whoOrEntries
      for (const e of entries) {
        this.operations.push({
          type: "authorize_account",
          who: e.who,
          transactions: e.transactions,
          bytes: e.bytes,
        })
      }
      return mockReceipt()
    })
  }

  authorizePreimage(contentHash: Uint8Array, maxSize: bigint): AuthCallBuilder {
    return new AuthCallBuilder(async () => {
      this.throwIfAuthFailure()
      this.operations.push({
        type: "authorize_preimage",
        contentHash,
        maxSize,
      })
      return mockReceipt()
    })
  }

  refreshAccountAuthorization(who: string): AuthCallBuilder {
    return new AuthCallBuilder(async () => {
      this.throwIfAuthFailure()
      this.operations.push({ type: "refresh_account_authorization", who })
      return mockReceipt()
    })
  }

  refreshPreimageAuthorization(contentHash: Uint8Array): AuthCallBuilder {
    return new AuthCallBuilder(async () => {
      this.throwIfAuthFailure()
      this.operations.push({
        type: "refresh_preimage_authorization",
        contentHash,
      })
      return mockReceipt()
    })
  }

  removeExpiredAccountAuthorization(who: string): CallBuilder {
    return new CallBuilder(async () => {
      this.operations.push({
        type: "remove_expired_account_authorization",
        who,
      })
      return mockReceipt()
    })
  }

  removeExpiredPreimageAuthorization(contentHash: Uint8Array): CallBuilder {
    return new CallBuilder(async () => {
      this.operations.push({
        type: "remove_expired_preimage_authorization",
        contentHash,
      })
      return mockReceipt()
    })
  }

  renew(block: number, index: number): CallBuilder {
    return new CallBuilder(async () => {
      this.operations.push({ type: "renew", block, index })
      return mockReceipt()
    })
  }

  estimateAuthorization(dataSize: number): {
    transactions: number
    bytes: number
  } {
    return {
      transactions:
        Math.ceil(dataSize / this.config.defaultChunkSize) +
        (this.config.createManifest ? 1 : 0),
      bytes: dataSize,
    }
  }

  /**
   * Mock implementation of estimateUpload. Doesn't query a chain (no chain in
   * the mock), so `skipExisting`/`alreadyStored` is always empty; only input
   * duplicates are surfaced. Mirrors the real client's shape — including the
   * streaming `estimateUpload(source)` overload, which chunks offline.
   */
  async estimateUpload(
    input: UploadItem[] | BlobSource,
    options: UploadEstimateOptions = {},
  ): Promise<StreamEstimate> {
    if (Array.isArray(input)) {
      const cids = await Promise.all(
        input.map((item) =>
          calculateCid(
            item.data,
            item.codec ?? CidCodec.Raw,
            item.hashAlgo ?? HashAlgorithm.Blake2b256,
          ),
        ),
      )
      const sizes = input.map((i) => i.data.length)
      const offsets: number[] = []
      let total = 0
      for (const s of sizes) {
        offsets.push(total)
        total += s
      }
      const plan: ChunkPlan = {
        chunkCids: cids,
        chunkSizes: sizes,
        offsets,
        codecs: input.map((it) => it.codec ?? CidCodec.Raw),
        hashAlgos: input.map((it) => it.hashAlgo ?? HashAlgorithm.Blake2b256),
        totalSize: total,
        chunkSize: 0,
      }
      return { ...this.assembleEstimate(cids, sizes, options), plan }
    }
    const plan = await this.preparer.planStream(input)
    const cids: CID[] = [...plan.chunkCids]
    const sizes: number[] = [...plan.chunkSizes]
    if (plan.rootCid && plan.manifestData) {
      cids.push(plan.rootCid)
      sizes.push(plan.manifestData.length)
    }
    return { ...this.assembleEstimate(cids, sizes, options), plan }
  }

  /** Offline dedup + assembly (no chain → `alreadyStored` always empty). */
  private assembleEstimate(
    cids: CID[],
    sizes: number[],
    options: UploadEstimateOptions,
  ): UploadEstimate {
    const dedupInput = options.dedupInput ?? true
    const hashesHex = cids.map(cidToContentHashHex)
    const seen = new Map<string, number>()
    const duplicateIndices: number[] = []
    if (dedupInput) {
      for (let i = 0; i < cids.length; i++) {
        const h = hashesHex[i] as string
        if (seen.has(h)) duplicateIndices.push(i)
        else seen.set(h, i)
      }
    }
    const dupSet = new Set(duplicateIndices)
    const toUpload: number[] = []
    let bytes = 0n
    const itemsOut: UploadEstimateItem[] = new Array(cids.length)
    for (let i = 0; i < cids.length; i++) {
      const dup = dupSet.has(i)
      itemsOut[i] = {
        index: i,
        cid: cids[i] as CID,
        bytes: sizes[i] as number,
        ...(dup ? { skipReason: "duplicate_input" as const } : {}),
      }
      if (!dup) {
        toUpload.push(i)
        bytes += BigInt(sizes[i] as number)
      }
    }
    return {
      total: cids.length,
      items: itemsOut,
      transactions: toUpload.length,
      bytes,
      duplicateIndices,
      alreadyStored: [],
      toUpload,
    }
  }
}
