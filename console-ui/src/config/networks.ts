export interface MonitoringLinks {
  /** Parity SRE Grafana dashboard for this chain. */
  grafana?: string;
  /** Sentry dashboard for product-side telemetry on this chain. */
  sentry?: string;
  /** Sentry drill-down: deploy.storage phase (per-deploy Bulletin write). */
  sentryStorageSpan?: string;
  /** Sentry drill-down: deploy.chunk-upload phase (per-chunk write latency). */
  sentryChunkUploadSpan?: string;
  /** Sentry drill-down: deploy.chain-probe phase (cache-check RPC reads). */
  sentryChainProbeSpan?: string;
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
const GRAFANA_COMMON_QS =
  "orgId=1&from=now-6h&to=now&timezone=utc&var-data_source=PC96415006F908B67";
const SENTRY_BULLETIN_DEPLOY_HEALTH =
  "https://paritytech.sentry.io/dashboard/1669817/?project=4511093597405264&project=4511298552135760&statsPeriod=24h";
const SENTRY_PHASE_WIDGET_BASE =
  "https://paritytech.sentry.io/dashboard/1669817/widget/8/?project=4511093597405264&project=4511298552135760&sort=-avg%28span.duration%29&statsPeriod=24h";
const TELEMETRY_POLKADOT = "https://telemetry.polkadot.io/";

function sentrySpanLink(spanOp: string): string {
  return `${SENTRY_PHASE_WIDGET_BASE}&query=${encodeURIComponent(`span.op:${spanOp}`)}`;
}

const SENTRY_STORAGE_SPAN = sentrySpanLink("deploy.storage");
const SENTRY_CHUNK_UPLOAD_SPAN = sentrySpanLink("deploy.chunk-upload");
const SENTRY_CHAIN_PROBE_SPAN = sentrySpanLink("deploy.chain-probe");

function grafanaLink(chain: string, node?: string): string {
  const params = [`var-chain=${chain}`, GRAFANA_COMMON_QS];
  if (node) params.push(`var-node=${node}`);
  return `${GRAFANA_OPERATION_HEALTH}?${params.join("&")}`;
}

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
      grafana: grafanaLink("bulletin-westend"),
      sentry: SENTRY_BULLETIN_DEPLOY_HEALTH,
      sentryStorageSpan: SENTRY_STORAGE_SPAN,
      sentryChunkUploadSpan: SENTRY_CHUNK_UPLOAD_SPAN,
      sentryChainProbeSpan: SENTRY_CHAIN_PROBE_SPAN,
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
      grafana: grafanaLink("bulletin-paseo"),
      sentry: SENTRY_BULLETIN_DEPLOY_HEALTH,
      sentryStorageSpan: SENTRY_STORAGE_SPAN,
      sentryChunkUploadSpan: SENTRY_CHUNK_UPLOAD_SPAN,
      sentryChainProbeSpan: SENTRY_CHAIN_PROBE_SPAN,
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
      grafana: grafanaLink(
        "next-bulletin-paseo",
        "paseo-bulletin-next-collator-node-0",
      ),
      sentry: SENTRY_BULLETIN_DEPLOY_HEALTH,
      sentryStorageSpan: SENTRY_STORAGE_SPAN,
      sentryChunkUploadSpan: SENTRY_CHUNK_UPLOAD_SPAN,
      sentryChainProbeSpan: SENTRY_CHAIN_PROBE_SPAN,
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
      grafana: grafanaLink("bulletin-polkadot"),
      sentry: SENTRY_BULLETIN_DEPLOY_HEALTH,
      sentryStorageSpan: SENTRY_STORAGE_SPAN,
      sentryChunkUploadSpan: SENTRY_CHUNK_UPLOAD_SPAN,
      sentryChainProbeSpan: SENTRY_CHAIN_PROBE_SPAN,
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
