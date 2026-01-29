// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Transaction submission for Bulletin Chain operations
 */

import { PolkadotSigner, TypedApi } from 'polkadot-api';
import { Authorization, BulletinError } from './types.js';

/**
 * Transaction receipt from a successful submission
 */
export interface TransactionReceipt {
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
export interface TransactionSubmitter {
  /** Submit a store transaction */
  submitStore(data: Uint8Array): Promise<TransactionReceipt>;

  /** Submit an authorize_account transaction */
  submitAuthorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
  ): Promise<TransactionReceipt>;

  /** Submit an authorize_preimage transaction */
  submitAuthorizePreimage(
    contentHash: Uint8Array,
    maxSize: bigint,
  ): Promise<TransactionReceipt>;

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
export class PAPITransactionSubmitter implements TransactionSubmitter {
  constructor(
    private api: any,
    private signer: PolkadotSigner,
  ) {}

  async submitStore(data: Uint8Array): Promise<TransactionReceipt> {
    try {
      const tx = this.api.tx.TransactionStorage.store({ data });
      const result = await tx.signAndSubmit(this.signer);

      // Wait for finalization
      const finalized = await result.waitFor('finalized');

      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to submit store transaction: ${error as any}`,
        'TRANSACTION_FAILED',
        error,
      );
    }
  }

  async submitAuthorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
  ): Promise<TransactionReceipt> {
    try {
      const tx = this.api.tx.TransactionStorage.authorize_account({
        who,
        transactions,
        bytes,
      });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor('finalized');

      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to authorize account: ${error as any}`,
        'AUTHORIZATION_FAILED',
        error,
      );
    }
  }

  async submitAuthorizePreimage(
    contentHash: Uint8Array,
    maxSize: bigint,
  ): Promise<TransactionReceipt> {
    try {
      const tx = this.api.tx.TransactionStorage.authorize_preimage({
        content_hash: contentHash,
        max_size: maxSize,
      });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor('finalized');

      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to authorize preimage: ${error as any}`,
        'AUTHORIZATION_FAILED',
        error,
      );
    }
  }

  async submitRenew(block: number, index: number): Promise<TransactionReceipt> {
    try {
      const tx = this.api.tx.TransactionStorage.renew({ block, index });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor('finalized');

      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to renew: ${error as any}`,
        'TRANSACTION_FAILED',
        error,
      );
    }
  }

  async submitRefreshAccountAuthorization(who: string): Promise<TransactionReceipt> {
    try {
      const tx = this.api.tx.TransactionStorage.refresh_account_authorization({ who });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor('finalized');

      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to refresh authorization: ${error as any}`,
        'TRANSACTION_FAILED',
        error,
      );
    }
  }

  async submitRefreshPreimageAuthorization(contentHash: Uint8Array): Promise<TransactionReceipt> {
    try {
      const tx = this.api.tx.TransactionStorage.refresh_preimage_authorization({
        content_hash: contentHash,
      });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor('finalized');

      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to refresh preimage authorization: ${error as any}`,
        'TRANSACTION_FAILED',
        error,
      );
    }
  }

  async submitRemoveExpiredAccountAuthorization(who: string): Promise<TransactionReceipt> {
    try {
      const tx = this.api.tx.TransactionStorage.remove_expired_account_authorization({ who });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor('finalized');

      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to remove expired authorization: ${error as any}`,
        'TRANSACTION_FAILED',
        error,
      );
    }
  }

  async submitRemoveExpiredPreimageAuthorization(
    contentHash: Uint8Array,
  ): Promise<TransactionReceipt> {
    try {
      const tx = this.api.tx.TransactionStorage.remove_expired_preimage_authorization({
        content_hash: contentHash,
      });
      const result = await tx.signAndSubmit(this.signer);
      const finalized = await result.waitFor('finalized');

      return {
        blockHash: finalized.blockHash,
        txHash: finalized.txHash,
        blockNumber: finalized.blockNumber,
      };
    } catch (error) {
      throw new BulletinError(
        `Failed to remove expired preimage authorization: ${error as any}`,
        'TRANSACTION_FAILED',
        error,
      );
    }
  }
}
