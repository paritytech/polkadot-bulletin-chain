export interface MonitoringLinks {
  /** Parity SRE Grafana dashboard for this chain. */
  grafana?: string;
  /** Grafana Bitswap server insights dashboard (IPFS/Bitswap serve load). */
  bitswap?: string;
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
  /** Grafana Loki Explore link, pre-filtered to this chain's collator logs. */
  collatorLogs?: string;
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
const TELEMETRY_POLKADOT = "https://telemetry.polkadot.io/";

function telemetryLink(genesisHash: string): string {
  return `https://telemetry.polkadot.io/#list/${genesisHash}`;
}

const TELEMETRY_PASEO_NEXT_V2 = telemetryLink(
  "0x2761c95259d59e55ae3daf756c1413b46e45a5a2987299f8ef8e5d8e4776cbc4",
);

/**
 * Sentry Explore Traces deep-link, filtered to one Bulletin deploy.* span. Same
 * shape Sentry generates when you click a row in the Phase Breakdown widget.
 */
function sentrySpanLink(spanOp: string): string {
  const query = encodeURIComponent(
    `span.op:deploy.* !span.op:deploy !deploy.tag:e2e-* span.op:${spanOp}`,
  );
  const visualize = encodeURIComponent(
    JSON.stringify({
      chartType: 1,
      yAxes: [
        "avg(span.duration)",
        "p90(span.duration)",
        "p95(span.duration)",
        "count()",
      ],
    }),
  );
  return (
    "https://paritytech.sentry.io/explore/traces/" +
    "?field=span.op&field=span.duration" +
    "&groupBy=span.op&interval=15m&mode=samples" +
    "&project=4511093597405264&project=4511298552135760" +
    `&query=${query}` +
    "&sort=-span.duration&statsPeriod=24h" +
    `&visualize=${visualize}`
  );
}

const SENTRY_STORAGE_SPAN = sentrySpanLink("deploy.storage");
const SENTRY_CHUNK_UPLOAD_SPAN = sentrySpanLink("deploy.chunk-upload");
const SENTRY_CHAIN_PROBE_SPAN = sentrySpanLink("deploy.chain-probe");

function grafanaLink(chain: string, node?: string): string {
  const params = [`var-chain=${chain}`, GRAFANA_COMMON_QS];
  if (node) params.push(`var-node=${node}`);
  return `${GRAFANA_OPERATION_HEALTH}?${params.join("&")}`;
}

const GRAFANA_BITSWAP =
  "https://grafana.teleport.parity.io/d/bitswap-1/bitswap-server-insights";

// Bitswap insights uses its own `project=thanos` template var; `var-chain`
// matches operation-health, so the same chain string feeds both dashboards.
function bitswapLink(chain: string): string {
  return (
    `${GRAFANA_BITSWAP}?orgId=1&from=now-6h&to=now&timezone=utc` +
    `&var-project=thanos&var-chain=${chain}&var-nodename=All`
  );
}

function polkadotJsAppsLink(endpoint: string): string {
  return `https://polkadot.js.org/apps/?rpc=${encodeURIComponent(endpoint)}`;
}

const LOKI_DATASOURCE_UID = "P44F328058D1A830B";

/**
 * Grafana Loki Explore deep-link filtered to a chain's collator pods. Chain
 * value matches Loki's `chain=` label (set by SRE scrape config), which is
 * typically `bulletin-next-paseo`, `bulletin-paseo`, etc.
 */
function lokiLogsLink(chain: string): string {
  const panes = {
    dt9: {
      datasource: LOKI_DATASOURCE_UID,
      queries: [
        {
          refId: "A",
          expr: `{chain="${chain}"} |= \`\``,
          queryType: "range",
          datasource: { type: "loki", uid: LOKI_DATASOURCE_UID },
          editorMode: "builder",
          direction: "backward",
        },
      ],
      range: { from: "now-1h", to: "now" },
      compact: false,
    },
  };
  return (
    "https://grafana.teleport.parity.io/explore?schemaVersion=1" +
    `&panes=${encodeURIComponent(JSON.stringify(panes))}&orgId=1`
  );
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
      bitswap: bitswapLink("bulletin-westend"),
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
      bitswap: bitswapLink("bulletin-paseo"),
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
      bitswap: bitswapLink("next-bulletin-paseo"),
      sentry: SENTRY_BULLETIN_DEPLOY_HEALTH,
      sentryStorageSpan: SENTRY_STORAGE_SPAN,
      sentryChunkUploadSpan: SENTRY_CHUNK_UPLOAD_SPAN,
      sentryChainProbeSpan: SENTRY_CHAIN_PROBE_SPAN,
      telemetry: TELEMETRY_PASEO_NEXT_V2,
      polkadotJs: polkadotJsAppsLink("wss://paseo-bulletin-next-rpc.polkadot.io"),
      collatorLogs: lokiLogsLink("bulletin-next-paseo"),
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
      bitswap: bitswapLink("bulletin-polkadot"),
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

export const DEFAULT_NETWORK = "paseo-next-v2";

// External Web3 Storage console; the in-app mode was removed in favour of this link.
export const WEB3_STORAGE_URL = "https://paritytech.github.io/web3-storage";
