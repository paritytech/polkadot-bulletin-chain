export interface Network {
  id: string;
  name: string;
  endpoints: string[];
  lightClient: boolean;
  chainSpec?: string;
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
  previewnet: {
    id: "previewnet",
    name: "Bulletin Previewnet",
    endpoints: ["wss://previewnet.substrate.dev/bulletin"],
    lightClient: false,
  },
  polkadot: {
    id: "polkadot",
    name: "Bulletin Polkadot (not released yet)",
    endpoints: [],
    lightClient: false,
  },
};

export const WEB3_STORAGE_NETWORKS: Record<string, Network> = {
  local: {
    id: "local",
    name: "Local Dev",
    endpoints: ["ws://localhost:2222"],
    lightClient: false,
  },
  westend: {
    id: "westend",
    name: "Web3 Westend (not released yet)",
    endpoints: [],
    lightClient: false,
  },
};

export const DEFAULT_NETWORKS = {
  bulletin: "paseo",
  web3storage: "local",
} as const;
