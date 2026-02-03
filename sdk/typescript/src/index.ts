// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Bulletin SDK for TypeScript/JavaScript
 *
 * Off-chain client SDK for Polkadot Bulletin Chain that simplifies data storage
 * with automatic chunking, authorization management, and DAG-PB manifest generation.
 *
 * @packageDocumentation
 */

export * from './types.js';
export * from './chunker.js';
export * from './dag.js';
export * from './utils.js';
export * from './client.js';
export * from './async-client.js';
export * from './mock-client.js';

export { CID } from 'multiformats/cid';

/**
 * SDK version
 */
export const VERSION = '0.1.0';
