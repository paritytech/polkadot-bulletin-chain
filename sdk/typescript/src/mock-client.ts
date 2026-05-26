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
import {
  AuthCallBuilder,
  type AuthorizeAccountEntry,
  type BulletinClientInterface,
  CallBuilder,
  type TransactionReceipt,
  UploadBuilder,
  UploadFileBuilder,
} from "./client.js"
import {
  BulletinError,
  type ChunkerConfig,
  CidCodec,
  type ClientConfig,
  ErrorCode,
  HashAlgorithm,
  type ResolvedClientConfig,
  resolveClientConfig,
  type UploadCallback,
  type UploadEstimate,
  type UploadEstimateItem,
  type UploadFileResult,
  type UploadItem,
  type UploadResult,
  UploadStatus,
  type WaitFor,
} from "./types.js"
import { calculateCid } from "./utils.js"
import { Binary } from "polkadot-api"

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
 * import { MockBulletinClient } from '@parity/bulletin-sdk';
 *
 * // Create mock client
 * const client = new MockBulletinClient();
 *
 * // Store data (no blockchain required)
 * const result = await client.store(data).send();
 * console.log('Mock CID:', result.cid.toString());
 *
 * // Check what operations were performed
 * const ops = client.getOperations();
 * expect(ops).toHaveLength(1);
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

  uploadFile(data: Uint8Array): UploadFileBuilder {
    return new UploadFileBuilder(
      (d, wf, oe, _cc, ca) => this.uploadFileImpl(d, wf, oe, ca),
      data,
    )
  }

  upload(items: UploadItem[]): UploadBuilder {
    return new UploadBuilder(
      (its, _wf, oe, ca) => this.uploadItemsImpl(its, oe, ca),
      (its) => this.estimateUpload(its),
      items,
    )
  }

  private async uploadFileImpl(
    data: Uint8Array,
    _waitFor: WaitFor,
    onEvent: UploadCallback | undefined,
    checkAuth: boolean,
  ): Promise<UploadFileResult> {
    if (data.length === 0) {
      throw new BulletinError("Data cannot be empty", ErrorCode.EMPTY_DATA)
    }
    const { cids } = await this.uploadItemsImpl([{ data }], onEvent, checkAuth)
    return { cid: cids[0]! }
  }

  private async uploadItemsImpl(
    items: UploadItem[],
    onEvent: UploadCallback | undefined,
    checkAuth: boolean,
  ): Promise<UploadResult> {
    if (items.length === 0) {
      throw new BulletinError(
        "upload() requires at least one item",
        ErrorCode.EMPTY_DATA,
      )
    }
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
    // Compute all CIDs in parallel (matches the real client's behavior).
    const cids: CID[] = await Promise.all(
      items.map((item) =>
        calculateCid(
          item.data,
          item.codec ?? CidCodec.Raw,
          item.hashAlgo ?? HashAlgorithm.Blake2b256,
        ),
      ),
    )
    for (let i = 0; i < items.length; i++) {
      const item = items[i] as UploadItem
      const cid = cids[i]!
      this.operations.push({
        type: "store",
        dataSize: item.data.length,
        cid: cid.toString(),
      })
      onEvent?.({
        type: UploadStatus.ItemStarted,
        index: i,
        total: items.length,
        cid,
      })
      onEvent?.({
        type: UploadStatus.ItemFinalized,
        index: i,
        total: items.length,
        cid,
        blockHash: MOCK_BLOCK_HASH,
        blockNumber: 1,
      })
    }
    return { cids }
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
   * Mock implementation of estimateUpload. Doesn't query a chain (no
   * chain in the mock), so `skipExisting` only ever surfaces input
   * duplicates. Mirrors the real client's shape so consumers can use the
   * same code path under both.
   */
  async estimateUpload(items: UploadItem[]): Promise<UploadEstimate> {
    const itemCids = await Promise.all(
      items.map((item) =>
        calculateCid(
          item.data,
          item.codec ?? CidCodec.Raw,
          item.hashAlgo ?? HashAlgorithm.Blake2b256,
        ),
      ),
    )
    const hashesHex = itemCids.map((cid) => Binary.toHex(cid.multihash.digest))
    const seen = new Map<string, number>()
    const duplicateIndices: number[] = []
    for (let i = 0; i < items.length; i++) {
      const h = hashesHex[i] as string
      if (seen.has(h)) duplicateIndices.push(i)
      else seen.set(h, i)
    }
    const dupSet = new Set(duplicateIndices)
    const toUpload: number[] = []
    let bytes = 0n
    const itemsOut: UploadEstimateItem[] = new Array(items.length)
    for (let i = 0; i < items.length; i++) {
      const item = items[i] as UploadItem
      const dup = dupSet.has(i)
      itemsOut[i] = {
        index: i,
        cid: itemCids[i] as CID,
        bytes: item.data.length,
        ...(dup ? { skipReason: "duplicate_input" as const } : {}),
      }
      if (!dup) {
        toUpload.push(i)
        bytes += BigInt(item.data.length)
      }
    }
    return {
      total: items.length,
      items: itemsOut,
      transactions: toUpload.length,
      bytes,
      duplicateIndices,
      alreadyStored: [],
      toUpload,
    }
  }
}
