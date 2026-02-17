import { createClient, PolkadotClient, TypedApi } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws-provider/web";
import { getSmProvider } from "polkadot-api/sm-provider";
import { startFromWorker } from "polkadot-api/smoldot/from-worker";
import { BehaviorSubject, map, shareReplay, combineLatest } from "rxjs";
import { bind } from "@react-rxjs/core";
import { bulletin_westend, bulletin_paseo, bulletin_dotspark } from "@polkadot-api/descriptors";

export type NetworkId = "local" | "westend" | "polkadot" | "paseo" | "dotspark";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const DESCRIPTORS: Partial<Record<NetworkId, any>> = {
  local: bulletin_westend,
  westend: bulletin_westend,
  paseo: bulletin_paseo,
  dotspark: bulletin_dotspark,
};

export interface Network {
  id: NetworkId;
  name: string;
  endpoints: string[];
  lightClient: boolean;
  chainSpec?: string;
}

export const NETWORKS: Record<NetworkId, Network> = {
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
    name: "Bulletin Paseo",
    endpoints: ["wss://paseo-bulletin-rpc.polkadot.io"],
    lightClient: false,
  },
  dotspark: {
    id: "dotspark",
    name: "Bulletin (Prototypes dotspark)",
    endpoints: ["wss://bulletin.dotspark.app"],
    lightClient: false,
  },
  polkadot: {
    id: "polkadot",
    name: "Bulletin Polkadot (not released yet)",
    endpoints: [],
    lightClient: false,
  },
};

export interface ChainState {
  network: Network;
  status: "disconnected" | "connecting" | "connected" | "error";
  error?: string;
  client?: PolkadotClient;
  // Using bulletin_westend as the base type; all bulletin chains share the same core pallets
  api?: TypedApi<typeof bulletin_westend>;
  blockNumber?: number;
  chainName?: string;
  tokenSymbol?: string;
  tokenDecimals?: number;
  ss58Format?: number;
}

const initialNetwork = NETWORKS.paseo;

const networkSubject = new BehaviorSubject<Network>(initialNetwork);
const statusSubject = new BehaviorSubject<ChainState["status"]>("disconnected");
const errorSubject = new BehaviorSubject<string | undefined>(undefined);
const clientSubject = new BehaviorSubject<PolkadotClient | undefined>(undefined);
const apiSubject = new BehaviorSubject<TypedApi<typeof bulletin_westend> | undefined>(undefined);
const blockNumberSubject = new BehaviorSubject<number | undefined>(undefined);
const chainInfoSubject = new BehaviorSubject<{
  chainName?: string;
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

export async function connectToNetwork(networkId: NetworkId): Promise<void> {
  const network = NETWORKS[networkId];
  if (!network) {
    throw new Error(`Unknown network: ${networkId}`);
  }
  if (network.endpoints.length === 0) {
    throw new Error(`Network ${network.name} has no endpoints available`);
  }

  // Disconnect existing client
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
      provider = getWsProvider(network.endpoints[0]!);
    }

    const client = createClient(provider);
    clientSubject.next(client);

    const descriptor = DESCRIPTORS[networkId] ?? bulletin_westend;
    const api = client.getTypedApi(descriptor) as TypedApi<typeof bulletin_westend>;
    apiSubject.next(api);

    // Get chain info from runtime constants (async)
    try {
      const version = await api.constants.System.Version();
      const ss58Format = await api.constants.System.SS58Prefix();

      chainInfoSubject.next({
        chainName: version.spec_name,
        tokenSymbol: "DOT",
        tokenDecimals: 10,
        ss58Format,
      });
    } catch {
      // Constants may not be available immediately
      chainInfoSubject.next({
        tokenSymbol: "DOT",
        tokenDecimals: 10,
      });
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
  networkSubject,
  statusSubject,
  errorSubject,
  clientSubject,
  apiSubject,
  blockNumberSubject,
  chainInfoSubject,
]).pipe(
  map(([network, status, error, client, api, blockNumber, chainInfo]) => ({
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
  network: initialNetwork,
  status: "disconnected" as const,
  error: undefined,
  client: undefined,
  api: undefined,
  blockNumber: undefined,
  chainName: undefined,
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
