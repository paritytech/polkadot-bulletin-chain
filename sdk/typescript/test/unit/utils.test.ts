// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

import { describe, it, expect } from 'vitest';
import { validateChunkSize } from '../../src/utils';

describe('Utils', () => {
  describe('validateChunkSize', () => {
    it('should validate valid chunk sizes', () => {
      expect(() => validateChunkSize(1024 * 1024)).not.toThrow(); // 1 MiB
      expect(() => validateChunkSize(8 * 1024 * 1024)).not.toThrow(); // 8 MiB (MAX_CHUNK_SIZE)
    });

    it('should reject zero size', () => {
      expect(() => validateChunkSize(0)).toThrow();
    });

    it('should reject negative size', () => {
      expect(() => validateChunkSize(-1)).toThrow();
    });

    it('should reject size exceeding maximum', () => {
      expect(() => validateChunkSize(10 * 1024 * 1024)).toThrow();
    });
  });
});
