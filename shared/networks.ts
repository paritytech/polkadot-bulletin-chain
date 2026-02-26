/**
 * Shared network configuration for Polkadot Bulletin Chain.
 *
 * Source of truth: networks.json
 * This TypeScript module provides typed access to the JSON config.
 *
 * Used by:
 * - Console UI
 * - TypeScript SDK
 * - Documentation examples
 *
 * For Rust SDK, use the JSON file directly or generate Rust code from it.
 */

import networksConfig from "./networks.json";

export interface Network {
  id: string;
  name: string;
  endpoints: string[];
  lightClient: boolean;
  chainSpec?: string;
}

export interface StorageTypeConfig {
  defaultNetwork: string;
  networks: Record<string, Network>;
}

export interface NetworkConfig {
  bulletin: StorageTypeConfig;
  web3storage: StorageTypeConfig;
}

// Type-safe access to the JSON config
const config = networksConfig as NetworkConfig;

/**
 * All available Bulletin Chain networks.
 */
export const BULLETIN_NETWORKS = config.bulletin.networks;

/**
 * Web3 Storage networks (experimental).
 */
export const WEB3_STORAGE_NETWORKS = config.web3storage.networks;

/**
 * Default network for each storage type.
 */
export const DEFAULT_NETWORKS = {
  bulletin: config.bulletin.defaultNetwork,
  web3storage: config.web3storage.defaultNetwork,
} as const;

/**
 * Get the default endpoint for a network.
 */
export function getDefaultEndpoint(networkId: string): string | undefined {
  const network = BULLETIN_NETWORKS[networkId] ?? WEB3_STORAGE_NETWORKS[networkId];
  return network?.endpoints[0];
}

/**
 * Get a network by ID (checks both bulletin and web3storage).
 */
export function getNetwork(networkId: string): Network | undefined {
  return BULLETIN_NETWORKS[networkId] ?? WEB3_STORAGE_NETWORKS[networkId];
}

/**
 * Common endpoint constants for quick access.
 */
export const ENDPOINTS = {
  LOCAL: BULLETIN_NETWORKS.local?.endpoints[0] ?? "ws://localhost:10000",
  WESTEND: BULLETIN_NETWORKS.westend?.endpoints[0] ?? "wss://westend-bulletin-rpc.polkadot.io",
  PASEO: BULLETIN_NETWORKS.paseo?.endpoints[0] ?? "wss://paseo-bulletin-rpc.polkadot.io",
  PREVIEWNET: BULLETIN_NETWORKS.previewnet?.endpoints[0] ?? "wss://previewnet.substrate.dev/bulletin",
} as const;

// Re-export the raw config for direct access
export { networksConfig };
