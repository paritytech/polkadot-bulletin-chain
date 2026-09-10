export interface Network {
  id: string;
  rpc: string;
}

export const NETWORKS: Record<string, Network> = {
  "paseo-next-v2": {
    id: "paseo-next-v2",
    rpc: "wss://paseo-bulletin-next-rpc.polkadot.io",
  },
  paseo: {
    id: "paseo",
    rpc: "wss://paseo-bulletin-rpc.polkadot.io",
  },
  westend: {
    id: "westend",
    rpc: "wss://westend-bulletin-rpc.polkadot.io",
  },
  polkadot: {
    id: "polkadot",
    rpc: "wss://bulletin-rpc.polkadot.io",
  },
};

export function resolveNetwork(id: string | undefined): Network {
  const key = id ?? "paseo-next-v2";
  const net = NETWORKS[key];
  if (!net) throw new Error(`unknown PROBE_NETWORK=${key}`);
  return net;
}
