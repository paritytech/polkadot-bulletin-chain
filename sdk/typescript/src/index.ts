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
export * from "./async-client.js"
export * from "./chunker.js"
export * from "./client.js"
export * from "./dag.js"
export * from "./mock-client.js"
export * from "./types.js"
export * from "./utils.js"

/**
 * SDK version
 */
export const VERSION = "0.1.0"
