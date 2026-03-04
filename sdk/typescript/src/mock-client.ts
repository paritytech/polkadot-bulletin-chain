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

import type { Binary } from "polkadot-api"
import {
  type AsyncClientConfig,
  StoreBuilder,
  type StoreExecutor,
  type TransactionReceipt,
} from "./async-client.js"
import {
  BulletinError,
  CidCodec,
  DEFAULT_STORE_OPTIONS,
  type ProgressCallback,
  type StoreOptions,
  type StoreResult,
} from "./types.js"
import { calculateCid, estimateAuthorization } from "./utils.js"

/**
 * Configuration for the mock Bulletin client
 */
export interface MockClientConfig extends AsyncClientConfig {
  /** Simulate authorization failures (for testing error paths) */
  simulateAuthFailure?: boolean
  /** Simulate storage failures (for testing error paths) */
  simulateStorageFailure?: boolean
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
  | { type: "remove_expired_account_authorization"; who: string }
  | {
      type: "remove_expired_preimage_authorization"
      contentHash: Uint8Array
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
 * import { MockBulletinClient } from '@bulletin/sdk';
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
export class MockBulletinClient implements StoreExecutor {
  /** Client configuration */
  public config: Required<
    Omit<MockClientConfig, "simulateAuthFailure" | "simulateStorageFailure">
  > & {
    simulateAuthFailure: boolean
    simulateStorageFailure: boolean
  }
  /** Operations performed (for testing verification) */
  private operations: MockOperation[] = []
  /** Account for authorization checks (optional) */
  private account?: string

  /**
   * Create a new mock client with optional configuration
   */
  constructor(config?: Partial<MockClientConfig>) {
    this.config = {
      defaultChunkSize: config?.defaultChunkSize ?? 1024 * 1024, // 1 MiB
      createManifest: config?.createManifest ?? true,
      chunkingThreshold: config?.chunkingThreshold ?? 2 * 1024 * 1024, // 2 MiB
      useSudo: config?.useSudo ?? false,
      simulateAuthFailure: config?.simulateAuthFailure ?? false,
      simulateStorageFailure: config?.simulateStorageFailure ?? false,
    }
  }

  /**
   * Set the account for authorization checks
   */
  withAccount(account: string): this {
    this.account = account
    return this
  }

  /**
   * Get the account set for authorization checks
   */
  getAccount(): string | undefined {
    return this.account
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
   * Store data using builder pattern
   *
   * @param data - Data to store (PAPI Binary or Uint8Array)
   */
  store(data: Binary | Uint8Array): StoreBuilder {
    return new StoreBuilder(this, data)
  }

  /**
   * Store data with custom options (internal, used by builder)
   */
  async storeWithOptions(
    data: Binary | Uint8Array,
    options?: StoreOptions,
    _progressCallback?: ProgressCallback,
  ): Promise<StoreResult> {
    // Convert Binary to Uint8Array if needed
    const dataBytes = data instanceof Uint8Array ? data : data.asBytes()

    if (dataBytes.length === 0) {
      throw new BulletinError("Data cannot be empty", "EMPTY_DATA")
    }

    // Simulate authorization failure
    if (this.config.simulateAuthFailure) {
      throw new BulletinError(
        "Insufficient authorization: need 100 bytes, have 0 bytes",
        "INSUFFICIENT_AUTHORIZATION",
        { need: 100, available: 0 },
      )
    }

    // Simulate storage failure
    if (this.config.simulateStorageFailure) {
      throw new BulletinError("Simulated storage failure", "TRANSACTION_FAILED")
    }

    const opts = { ...DEFAULT_STORE_OPTIONS, ...options }

    const cidCodec = opts.cidCodec ?? CidCodec.Raw
    const hashAlgorithm =
      opts.hashingAlgorithm ?? DEFAULT_STORE_OPTIONS.hashingAlgorithm

    const cid = await calculateCid(dataBytes, cidCodec, hashAlgorithm)

    // Record the operation
    this.operations.push({
      type: "store",
      dataSize: dataBytes.length,
      cid: cid.toString(),
    })

    // Return a mock receipt
    return {
      cid,
      size: dataBytes.length,
      blockNumber: 1,
    }
  }

  /**
   * Authorize an account to store data
   */
  async authorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
  ): Promise<TransactionReceipt> {
    if (this.config.simulateAuthFailure) {
      throw new BulletinError(
        "Simulated authorization failure",
        "AUTHORIZATION_FAILED",
      )
    }

    this.operations.push({
      type: "authorize_account",
      who,
      transactions,
      bytes,
    })

    return {
      blockHash:
        "0x0000000000000000000000000000000000000000000000000000000000000001",
      txHash:
        "0x0000000000000000000000000000000000000000000000000000000000000002",
      blockNumber: 1,
    }
  }

  /**
   * Authorize a preimage to be stored
   */
  async authorizePreimage(
    contentHash: Uint8Array,
    maxSize: bigint,
  ): Promise<TransactionReceipt> {
    if (this.config.simulateAuthFailure) {
      throw new BulletinError(
        "Simulated authorization failure",
        "AUTHORIZATION_FAILED",
      )
    }

    this.operations.push({
      type: "authorize_preimage",
      contentHash,
      maxSize,
    })

    return {
      blockHash:
        "0x0000000000000000000000000000000000000000000000000000000000000001",
      txHash:
        "0x0000000000000000000000000000000000000000000000000000000000000002",
      blockNumber: 1,
    }
  }

  /**
   * Refresh an account authorization (mock)
   */
  async refreshAccountAuthorization(who: string): Promise<TransactionReceipt> {
    if (this.config.simulateAuthFailure) {
      throw new BulletinError(
        "Simulated authorization failure",
        "AUTHORIZATION_FAILED",
      )
    }

    this.operations.push({
      type: "refresh_account_authorization",
      who,
    })

    return {
      blockHash:
        "0x0000000000000000000000000000000000000000000000000000000000000001",
      txHash:
        "0x0000000000000000000000000000000000000000000000000000000000000002",
      blockNumber: 1,
    }
  }

  /**
   * Refresh a preimage authorization (mock)
   */
  async refreshPreimageAuthorization(
    contentHash: Uint8Array,
  ): Promise<TransactionReceipt> {
    if (this.config.simulateAuthFailure) {
      throw new BulletinError(
        "Simulated authorization failure",
        "AUTHORIZATION_FAILED",
      )
    }

    this.operations.push({
      type: "refresh_preimage_authorization",
      contentHash,
    })

    return {
      blockHash:
        "0x0000000000000000000000000000000000000000000000000000000000000001",
      txHash:
        "0x0000000000000000000000000000000000000000000000000000000000000002",
      blockNumber: 1,
    }
  }

  /**
   * Remove an expired account authorization (mock)
   */
  async removeExpiredAccountAuthorization(
    who: string,
  ): Promise<TransactionReceipt> {
    this.operations.push({
      type: "remove_expired_account_authorization",
      who,
    })

    return {
      blockHash:
        "0x0000000000000000000000000000000000000000000000000000000000000001",
      txHash:
        "0x0000000000000000000000000000000000000000000000000000000000000002",
      blockNumber: 1,
    }
  }

  /**
   * Remove an expired preimage authorization (mock)
   */
  async removeExpiredPreimageAuthorization(
    contentHash: Uint8Array,
  ): Promise<TransactionReceipt> {
    this.operations.push({
      type: "remove_expired_preimage_authorization",
      contentHash,
    })

    return {
      blockHash:
        "0x0000000000000000000000000000000000000000000000000000000000000001",
      txHash:
        "0x0000000000000000000000000000000000000000000000000000000000000002",
      blockNumber: 1,
    }
  }

  /**
   * Estimate authorization needed for storing data
   */
  estimateAuthorization(dataSize: number): {
    transactions: number
    bytes: number
  } {
    return estimateAuthorization(
      dataSize,
      this.config.defaultChunkSize,
      this.config.createManifest,
    )
  }
}
