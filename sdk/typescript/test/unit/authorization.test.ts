// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, it, expect } from 'vitest';
import { AuthorizationManager } from '../../src/authorization';

describe('AuthorizationManager', () => {
  const manager = new AuthorizationManager();

  describe('estimateAuthorization', () => {
    it('should estimate authorization for 1 MB data', () => {
      const dataSize = 1024 * 1024; // 1 MB
      const estimate = manager.estimateAuthorization(dataSize);

      expect(estimate.transactions).toBe(1);
      expect(estimate.bytes).toBe(dataSize);
    });

    it('should estimate authorization for 10 MB data', () => {
      const dataSize = 10 * 1024 * 1024; // 10 MB
      const estimate = manager.estimateAuthorization(dataSize);

      expect(estimate.transactions).toBe(10);
      expect(estimate.bytes).toBe(dataSize);
    });

    it('should round up for fractional chunks', () => {
      const dataSize = 1.5 * 1024 * 1024; // 1.5 MB
      const estimate = manager.estimateAuthorization(dataSize);

      expect(estimate.transactions).toBe(2); // Rounds up to 2 chunks
      expect(estimate.bytes).toBe(dataSize);
    });

    it('should handle zero data size', () => {
      const estimate = manager.estimateAuthorization(0);

      expect(estimate.transactions).toBe(0);
      expect(estimate.bytes).toBe(0);
    });

    it('should handle data smaller than chunk size', () => {
      const dataSize = 512 * 1024; // 512 KB (< 1 MB)
      const estimate = manager.estimateAuthorization(dataSize);

      expect(estimate.transactions).toBe(1);
      expect(estimate.bytes).toBe(dataSize);
    });

    it('should handle very large data sizes', () => {
      const dataSize = 100 * 1024 * 1024; // 100 MB
      const estimate = manager.estimateAuthorization(dataSize);

      expect(estimate.transactions).toBe(100);
      expect(estimate.bytes).toBe(dataSize);
    });

    it('should handle exact chunk size multiples', () => {
      const dataSize = 5 * 1024 * 1024; // Exactly 5 MB
      const estimate = manager.estimateAuthorization(dataSize);

      expect(estimate.transactions).toBe(5);
      expect(estimate.bytes).toBe(dataSize);
    });

    it('should use custom chunk size', () => {
      const dataSize = 10 * 1024 * 1024; // 10 MB
      const chunkSize = 2 * 1024 * 1024; // 2 MB chunks

      const estimate = manager.estimateAuthorization(dataSize, chunkSize);

      expect(estimate.transactions).toBe(5); // 10 MB / 2 MB = 5 chunks
      expect(estimate.bytes).toBe(dataSize);
    });

    it('should round up with custom chunk size', () => {
      const dataSize = 11 * 1024 * 1024; // 11 MB
      const chunkSize = 2 * 1024 * 1024; // 2 MB chunks

      const estimate = manager.estimateAuthorization(dataSize, chunkSize);

      expect(estimate.transactions).toBe(6); // 11 MB / 2 MB = 5.5, rounds up to 6
      expect(estimate.bytes).toBe(dataSize);
    });
  });

  describe('calculateChunks', () => {
    it('should calculate chunk count correctly', () => {
      expect(manager.calculateChunks(1024 * 1024)).toBe(1);
      expect(manager.calculateChunks(5 * 1024 * 1024)).toBe(5);
      expect(manager.calculateChunks(1.5 * 1024 * 1024)).toBe(2);
      expect(manager.calculateChunks(0)).toBe(0);
    });

    it('should calculate chunk count with custom chunk size', () => {
      const chunkSize = 2 * 1024 * 1024; // 2 MB

      expect(manager.calculateChunks(2 * 1024 * 1024, chunkSize)).toBe(1);
      expect(manager.calculateChunks(10 * 1024 * 1024, chunkSize)).toBe(5);
      expect(manager.calculateChunks(11 * 1024 * 1024, chunkSize)).toBe(6);
    });
  });

  describe('Edge Cases', () => {
    it('should handle single byte', () => {
      const estimate = manager.estimateAuthorization(1);

      expect(estimate.transactions).toBe(1);
      expect(estimate.bytes).toBe(1);
    });

    it('should handle maximum safe integer', () => {
      const dataSize = Number.MAX_SAFE_INTEGER;
      const estimate = manager.estimateAuthorization(dataSize);

      expect(estimate.bytes).toBe(dataSize);
      expect(estimate.transactions).toBeGreaterThan(0);
    });

    it('should provide consistent estimates', () => {
      const dataSize = 5 * 1024 * 1024;

      const estimate1 = manager.estimateAuthorization(dataSize);
      const estimate2 = manager.estimateAuthorization(dataSize);

      expect(estimate1).toEqual(estimate2);
    });
  });
});
