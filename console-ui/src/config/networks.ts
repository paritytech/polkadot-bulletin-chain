// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

export interface Network {
  id: string;
  name: string;
  endpoints: string[];
  lightClient: boolean;
  chainSpec?: string;
  // HOP relay nodes for this network, exposing the public `hop_poolStatus`
  // JSON-RPC method over HTTPS POST. Polled by the HOP dashboard.
  hopNodes?: string[];
}

export const BULLETIN_NETWORKS: Record<string, Network> = {
  local: {
    id: "local",
    name: "Local Dev",
    endpoints: ["ws://localhost:10000"],
    lightClient: false,
  },
  westend: {
    id: "westend",
    name: "Bulletin Westend",
    endpoints: ["wss://westend-bulletin-rpc.polkadot.io"],
    lightClient: false,
  },
  paseo: {
    id: "paseo",
    name: "Bulletin Paseo Next",
    endpoints: ["wss://paseo-bulletin-rpc.polkadot.io"],
    lightClient: false,
  },
  "paseo-next-v2": {
    id: "paseo-next-v2",
    name: "Bulletin Paseo Next v2",
    endpoints: ["wss://paseo-bulletin-next-rpc.polkadot.io"],
    lightClient: false,
    hopNodes: [
      "wss://paseo-hop-next-0.polkadot.io",
      "wss://paseo-hop-next-1.polkadot.io",
    ],
  },
  summit: {
    id: "summit",
    name: "Bulletin Summit",
    endpoints: ["wss://summit-bulletin-rpc.polkadot.io"],
    lightClient: false,
    hopNodes: [
      "https://summit-hop-0.polkadot.io",
      "https://summit-hop-1.polkadot.io",
    ],
  },
  previewnet: {
    id: "previewnet",
    name: "Bulletin Previewnet",
    endpoints: ["wss://previewnet.substrate.dev/bulletin"],
    lightClient: false,
    hopNodes: [
      "wss://previewnet.substrate.dev/bulletin-hop-0",
      "wss://previewnet.substrate.dev/bulletin-hop-1",
    ],
  },
  polkadot: {
    id: "polkadot",
    name: "Bulletin Polkadot",
    endpoints: ["wss://bulletin-rpc.polkadot.io"],
    lightClient: false,
  },
  custom: {
    id: "custom",
    name: "Custom WS URL…",
    endpoints: [],
    lightClient: false,
  },
};

export const DEFAULT_NETWORK = "paseo-next-v2";

// External Web3 Storage console; the in-app mode was removed in favour of this link.
export const WEB3_STORAGE_URL = "https://paritytech.github.io/web3-storage";
