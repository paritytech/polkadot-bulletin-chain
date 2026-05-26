export interface MonitoringLinks {
  /** Parity SRE Grafana dashboard for this chain. */
  grafana?: string;
  /** Sentry dashboard for product-side telemetry on this chain. */
  sentry?: string;
  /** Substrate telemetry view. */
  telemetry?: string;
  /** PolkadotJS Apps deep-link. */
  polkadotJs?: string;
  /** Block explorer (Subscan etc.). */
  explorer?: string;
  /** Operational runbook. */
  runbook?: string;
}

export interface Network {
  id: string;
  name: string;
  endpoints: string[];
  lightClient: boolean;
  chainSpec?: string;
  monitoring?: MonitoringLinks;
}

const GRAFANA_OPERATION_HEALTH =
  "https://grafana.teleport.parity.io/d/cfdekwvxzxerkb/operation-health";
const SENTRY_BULLETIN_DEPLOY_HEALTH =
  "https://parityteh.sentry.io/dashboard/1669817/";
const TELEMETRY_POLKADOT = "https://telemetry.polkadot.io/";

function polkadotJsAppsLink(endpoint: string): string {
  return `https://polkadot.js.org/apps/?rpc=${encodeURIComponent(endpoint)}`;
}

export const BULLETIN_NETWORKS: Record<string, Network> = {
  local: {
    id: "local",
    name: "Local Dev",
    endpoints: ["ws://localhost:10000"],
    lightClient: false,
    monitoring: {
      polkadotJs: polkadotJsAppsLink("ws://localhost:10000"),
    },
  },
  westend: {
    id: "westend",
    name: "Bulletin Westend",
    endpoints: ["wss://westend-bulletin-rpc.polkadot.io"],
    lightClient: false,
    monitoring: {
      grafana: `${GRAFANA_OPERATION_HEALTH}?var-chain=bulletin-westend`,
      sentry: SENTRY_BULLETIN_DEPLOY_HEALTH,
      telemetry: TELEMETRY_POLKADOT,
      polkadotJs: polkadotJsAppsLink("wss://westend-bulletin-rpc.polkadot.io"),
    },
  },
  paseo: {
    id: "paseo",
    name: "Bulletin Paseo Next",
    endpoints: ["wss://paseo-bulletin-rpc.polkadot.io"],
    lightClient: false,
    monitoring: {
      grafana: `${GRAFANA_OPERATION_HEALTH}?var-chain=bulletin-paseo`,
      sentry: SENTRY_BULLETIN_DEPLOY_HEALTH,
      telemetry: TELEMETRY_POLKADOT,
      polkadotJs: polkadotJsAppsLink("wss://paseo-bulletin-rpc.polkadot.io"),
    },
  },
  "paseo-next-v2": {
    id: "paseo-next-v2",
    name: "Bulletin Paseo Next v2",
    endpoints: ["wss://paseo-bulletin-next-rpc.polkadot.io"],
    lightClient: false,
    monitoring: {
      grafana: `${GRAFANA_OPERATION_HEALTH}?var-chain=bulletin-paseo`,
      sentry: SENTRY_BULLETIN_DEPLOY_HEALTH,
      telemetry: TELEMETRY_POLKADOT,
      polkadotJs: polkadotJsAppsLink("wss://paseo-bulletin-next-rpc.polkadot.io"),
    },
  },
  previewnet: {
    id: "previewnet",
    name: "Bulletin Previewnet",
    endpoints: ["wss://previewnet.substrate.dev/bulletin"],
    lightClient: false,
    monitoring: {
      polkadotJs: polkadotJsAppsLink("wss://previewnet.substrate.dev/bulletin"),
    },
  },
  polkadot: {
    id: "polkadot",
    name: "Bulletin Polkadot",
    endpoints: ["wss://bulletin-rpc.polkadot.io"],
    lightClient: false,
    monitoring: {
      grafana: `${GRAFANA_OPERATION_HEALTH}?var-chain=bulletin-polkadot`,
      sentry: SENTRY_BULLETIN_DEPLOY_HEALTH,
      telemetry: TELEMETRY_POLKADOT,
      polkadotJs: polkadotJsAppsLink("wss://bulletin-rpc.polkadot.io"),
    },
  },
  custom: {
    id: "custom",
    name: "Custom WS URL…",
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
  bulletin: "paseo-next-v2",
  web3storage: "local",
} as const;
