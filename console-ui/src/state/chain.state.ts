import { createClient, PolkadotClient, TypedApi } from "polkadot-api";
import { getWsProvider } from "@polkadot-api/ws-provider";
import { getSmProvider } from "polkadot-api/sm-provider";
import { startFromWorker } from "polkadot-api/smoldot/from-worker";
import { BehaviorSubject, map, shareReplay, combineLatest } from "rxjs";
import { bind } from "@react-rxjs/core";
import { bulletin_westend, bulletin_paseo, web3_storage } from "@polkadot-api/descriptors";
import {
  BULLETIN_NETWORKS,
  WEB3_STORAGE_NETWORKS,
  DEFAULT_NETWORKS,
  type Network,
} from "../../../shared/networks";

export type StorageType = "bulletin" | "web3storage";

export type NetworkId = string;

// Re-export Network type for convenience
export type { Network };

export interface StorageConfig {
  id: StorageType;
  name: string;
  networks: Record<string, Network>;
  defaultNetwork: string;
}

export const STORAGE_CONFIGS: Record<StorageType, StorageConfig> = {
  bulletin: {
    id: "bulletin",
    name: "Bulletin",
    defaultNetwork: DEFAULT_NETWORKS.bulletin,
    networks: BULLETIN_NETWORKS,
  },
  web3storage: {
    id: "web3storage",
    name: "Web3 Storage",
    defaultNetwork: DEFAULT_NETWORKS.web3storage,
    networks: WEB3_STORAGE_NETWORKS,
  },
};

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const DESCRIPTORS: Record<string, Record<string, any>> = {
  bulletin: {
    local: bulletin_westend,
    westend: bulletin_westend,
    paseo: bulletin_paseo,
    previewnet: bulletin_westend,
  },
  web3storage: {
    local: web3_storage,
    westend: web3_storage,
  },
};

// No-op WebSocket that never connects. Used to silence the PAPI provider's
// internal reconnection loop after we switch away from a network.
// Without this, getSyncProvider keeps retrying with real WebSocket connections
// because client.destroy() doesn't fully stop pending reconnection attempts.
class NullWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  readyState = 3;
  constructor(_url: string | URL, _protocols?: string | string[]) {}
  addEventListener() {}
  removeEventListener() {}
  close() {}
  send() {}
  dispatchEvent() { return false; }
}

// Track the current provider's kill switch so we can silence its reconnection loop
let killCurrentProvider: (() => void) | null = null;

function createKillableWsProvider(endpoint: string) {
  let killed = false;

  // Proxy intercepts `new WebsocketClass(...)` and returns a NullWebSocket
  // once killed, preventing any real network connections from retry loops
  const wsClass = new Proxy(WebSocket, {
    construct(target, args: [string, string?]) {
      if (killed) return new NullWebSocket(args[0], args[1]) as unknown as WebSocket;
      return new target(args[0], args[1]);
    },
  });

  const provider = getWsProvider(endpoint, {
    websocketClass: wsClass as typeof WebSocket,
  });

  const kill = () => { killed = true; };
  return { provider, kill };
}

export interface ChainState {
  storageType: StorageType;
  network: Network;
  networks: Record<string, Network>;
  status: "disconnected" | "connecting" | "connected" | "error";
  error?: string;
  client?: PolkadotClient;
  // Using bulletin_westend as the base type; all bulletin chains share the same core pallets
  api?: TypedApi<typeof bulletin_westend>;
  blockNumber?: number;
  chainName?: string;
  specVersion?: number;
  tokenSymbol?: string;
  tokenDecimals?: number;
  ss58Format?: number;
}

const initialStorageType: StorageType = "bulletin";
const initialConfig = STORAGE_CONFIGS[initialStorageType];
const initialNetwork = initialConfig.networks[initialConfig.defaultNetwork]!;

const storageTypeSubject = new BehaviorSubject<StorageType>(initialStorageType);
const networksSubject = new BehaviorSubject<Record<string, Network>>(initialConfig.networks);
const networkSubject = new BehaviorSubject<Network>(initialNetwork);
const statusSubject = new BehaviorSubject<ChainState["status"]>("disconnected");
const errorSubject = new BehaviorSubject<string | undefined>(undefined);
const clientSubject = new BehaviorSubject<PolkadotClient | undefined>(undefined);
const apiSubject = new BehaviorSubject<TypedApi<typeof bulletin_westend> | undefined>(undefined);
const blockNumberSubject = new BehaviorSubject<number | undefined>(undefined);
const chainInfoSubject = new BehaviorSubject<{
  chainName?: string;
  specVersion?: number;
  tokenSymbol?: string;
  tokenDecimals?: number;
  ss58Format?: number;
}>({});
const sudoKeySubject = new BehaviorSubject<string | undefined>(undefined);

let smoldotWorker: Worker | null = null;

async function createSmoldotProvider(network: Network) {
  if (!smoldotWorker) {
    smoldotWorker = new Worker(
      new URL("polkadot-api/smoldot/worker", import.meta.url),
      { type: "module" }
    );
  }

  const smoldot = startFromWorker(smoldotWorker);
  const chainSpec = await fetch(`/chain-specs/${network.id}.json`).then(r => r.text());
  const chain = await smoldot.addChain({ chainSpec });

  return getSmProvider(chain);
}

export function switchStorageType(type: StorageType): void {
  const config = STORAGE_CONFIGS[type];
  storageTypeSubject.next(type);
  networksSubject.next(config.networks);
  connectToNetwork(config.defaultNetwork);
}

export async function connectToNetwork(networkId: NetworkId): Promise<void> {
  const networks = networksSubject.getValue();
  const network = networks[networkId];
  if (!network) {
    throw new Error(`Unknown network: ${networkId}`);
  }
  if (network.endpoints.length === 0) {
    throw new Error(`Network ${network.name} has no endpoints available`);
  }

  // Kill previous provider's reconnection loop and destroy client
  if (killCurrentProvider) {
    killCurrentProvider();
    killCurrentProvider = null;
  }
  const existingClient = clientSubject.getValue();
  if (existingClient) {
    existingClient.destroy();
  }

  networkSubject.next(network);
  statusSubject.next("connecting");
  errorSubject.next(undefined);
  apiSubject.next(undefined);
  blockNumberSubject.next(undefined);
  chainInfoSubject.next({});
  sudoKeySubject.next(undefined);

  try {
    let provider;

    if (network.lightClient && network.chainSpec) {
      provider = await createSmoldotProvider(network);
    } else {
      const killable = createKillableWsProvider(network.endpoints[0]!);
      provider = killable.provider;
      killCurrentProvider = killable.kill;
    }

    const client = createClient(provider);
    clientSubject.next(client);

    const descriptor = DESCRIPTORS[storageTypeSubject.getValue()]?.[networkId] ?? bulletin_westend;
    const api = client.getTypedApi(descriptor) as TypedApi<typeof bulletin_westend>;
    apiSubject.next(api);

    // Get chain info from runtime constants and RPC
    try {
      const [version, ss58Format, properties] = await Promise.all([
        api.constants.System.Version(),
        api.constants.System.SS58Prefix(),
        client._request<{ tokenSymbol?: string; tokenDecimals?: number }>("system_properties", []),
      ]);

      chainInfoSubject.next({
        chainName: version.spec_name,
        specVersion: version.spec_version,
        tokenSymbol: properties.tokenSymbol ?? "Unit",
        tokenDecimals: properties.tokenDecimals ?? 12,
        ss58Format,
      });
    } catch {
      // Constants may not be available immediately
      chainInfoSubject.next({});
    }

    // Get sudo key
    try {
      const sudoKey = await api.query.Sudo.Key.getValue();
      sudoKeySubject.next(sudoKey ?? undefined);
    } catch {
      // Sudo pallet may not be available
      sudoKeySubject.next(undefined);
    }

    // Subscribe to best block
    client.bestBlocks$.subscribe({
      next: (blocks) => {
        if (blocks.length > 0) {
          blockNumberSubject.next(blocks[0]!.number);
        }
      },
      error: (err) => {
        console.error("Block subscription error:", err);
      },
    });

    statusSubject.next("connected");
  } catch (err) {
    const message = err instanceof Error ? err.message : "Unknown error";
    errorSubject.next(message);
    statusSubject.next("error");
  }
}

export function disconnect(): void {
  if (killCurrentProvider) {
    killCurrentProvider();
    killCurrentProvider = null;
  }
  const client = clientSubject.getValue();
  if (client) {
    client.destroy();
  }
  clientSubject.next(undefined);
  apiSubject.next(undefined);
  blockNumberSubject.next(undefined);
  chainInfoSubject.next({});
  sudoKeySubject.next(undefined);
  statusSubject.next("disconnected");
}

// Combined chain state observable
const chainState$ = combineLatest([
  storageTypeSubject,
  networksSubject,
  networkSubject,
  statusSubject,
  errorSubject,
  clientSubject,
  apiSubject,
  blockNumberSubject,
  chainInfoSubject,
]).pipe(
  map(([storageType, networks, network, status, error, client, api, blockNumber, chainInfo]) => ({
    storageType,
    networks,
    network,
    status,
    error,
    client,
    api,
    blockNumber,
    ...chainInfo,
  })),
  shareReplay(1)
);

// React hooks
export const [useChainState] = bind(chainState$, {
  storageType: initialStorageType,
  networks: initialConfig.networks,
  network: initialNetwork,
  status: "disconnected" as const,
  error: undefined,
  client: undefined,
  api: undefined,
  blockNumber: undefined,
  chainName: undefined,
  specVersion: undefined,
  tokenSymbol: undefined,
  tokenDecimals: undefined,
  ss58Format: undefined,
});

export const [useNetwork] = bind(networkSubject);
export const [useConnectionStatus] = bind(statusSubject, "disconnected");
export const [useBlockNumber] = bind(blockNumberSubject, undefined);
export const [useApi] = bind(apiSubject, undefined);
export const [useClient] = bind(clientSubject, undefined);
export const [useSudoKey] = bind(sudoKeySubject, undefined);

// Direct access to subjects for non-React code
export const network$ = networkSubject.asObservable();
export const status$ = statusSubject.asObservable();
export const api$ = apiSubject.asObservable();
export const client$ = clientSubject.asObservable();
