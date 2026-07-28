// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

import { createClient, PolkadotClient, PolkadotSigner, TypedApi } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws";
import { getSmProvider } from "polkadot-api/sm-provider";
import { startFromWorker } from "polkadot-api/smoldot/from-worker";
import type { Chain } from "polkadot-api/smoldot";
import SmWorker from "polkadot-api/smoldot/worker?worker";
import { BehaviorSubject, map, shareReplay, combineLatest } from "rxjs";
import { bind } from "@react-rxjs/core";
import { bulletin_paseo_next_v2 } from "@polkadot-api/descriptors";
import {
  BULLETIN_NETWORKS,
  DEFAULT_NETWORK,
  type Network,
} from "../config/networks";
import { AsyncBulletinClient } from "@parity/bulletin-sdk";

export type NetworkId = string;

// Re-export Network type for convenience
export type { Network };

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
// Track the bestBlocks$ subscription so we can clean it up on disconnect/reconnect
let blockSubscription: { unsubscribe(): void } | null = null;

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
  network: Network;
  networks: Record<string, Network>;
  status: "disconnected" | "connecting" | "connected" | "error";
  error?: string;
  client?: PolkadotClient;
  // Base type is the newest live chain's descriptors; all bulletin chains
  // share the same core pallets, and older chains are guarded at runtime.
  api?: TypedApi<typeof bulletin_paseo_next_v2>;
  /** Active connection transport; "light-client" only on smoldot networks. */
  transport: Transport;
  blockNumber?: number;
  chainName?: string;
  specVersion?: number;
  tokenSymbol?: string;
  tokenDecimals?: number;
  ss58Format?: number;
}

const STORAGE_KEY_NETWORK = "bulletin-network";
const STORAGE_KEY_CUSTOM_URL = "bulletin-network-custom-url";
const STORAGE_KEY_TRANSPORT = "bulletin-transport";

export type Transport = "light-client" | "rpc";

function resolveTransport(network: Network): Transport {
  if (!network.lightClient || !network.chainSpec) return "rpc";
  return localStorage.getItem(STORAGE_KEY_TRANSPORT) === "rpc"
    ? "rpc"
    : "light-client";
}

export function setTransport(transport: Transport): void {
  localStorage.setItem(STORAGE_KEY_TRANSPORT, transport);
  connectToNetwork(networkSubject.getValue().id);
}

export function getCustomNetworkUrl(): string {
  return localStorage.getItem(STORAGE_KEY_CUSTOM_URL) ?? "";
}

export function clearCustomNetworkUrl(): void {
  localStorage.removeItem(STORAGE_KEY_CUSTOM_URL);
  const current = networkSubject.getValue();
  if (current.id === "custom") {
    connectToNetwork(DEFAULT_NETWORK);
  }
}

function loadInitialSelection(): Network {
  const savedNetwork = localStorage.getItem(STORAGE_KEY_NETWORK);
  const networkId = savedNetwork && BULLETIN_NETWORKS[savedNetwork] ? savedNetwork : DEFAULT_NETWORK;
  const baseNetwork = BULLETIN_NETWORKS[networkId]!;

  if (networkId === "custom") {
    const customUrl = localStorage.getItem(STORAGE_KEY_CUSTOM_URL);
    if (customUrl) {
      return { ...baseNetwork, endpoints: [customUrl] };
    }
  }

  return baseNetwork;
}

const initialNetwork = loadInitialSelection();

const networksSubject = new BehaviorSubject<Record<string, Network>>(BULLETIN_NETWORKS);
const networkSubject = new BehaviorSubject<Network>(initialNetwork);
const statusSubject = new BehaviorSubject<ChainState["status"]>("disconnected");
const errorSubject = new BehaviorSubject<string | undefined>(undefined);
const clientSubject = new BehaviorSubject<PolkadotClient | undefined>(undefined);
const apiSubject = new BehaviorSubject<TypedApi<typeof bulletin_paseo_next_v2> | undefined>(undefined);
const blockNumberSubject = new BehaviorSubject<number | undefined>(undefined);
const chainInfoSubject = new BehaviorSubject<{
  chainName?: string;
  specVersion?: number;
  tokenSymbol?: string;
  tokenDecimals?: number;
  ss58Format?: number;
}>({});
const sudoKeySubject = new BehaviorSubject<string | undefined>(undefined);
const transportSubject = new BehaviorSubject<Transport>(
  resolveTransport(initialNetwork),
);

// Smoldot worker lives for the app lifetime; chains are added/removed per network.
let smoldot: ReturnType<typeof startFromWorker> | null = null;

function getSmoldot() {
  if (!smoldot) smoldot = startFromWorker(new SmWorker());
  return smoldot;
}

function createSmoldotProvider(specs: { para: string; relay: string }) {
  let killed = false;
  const chains: Promise<Chain>[] = [];

  // The provider may invoke the factory again after a halt; each invocation
  // must return a fresh chain (smoldot dedups the relay internally).
  const provider = getSmProvider(async () => {
    const sd = getSmoldot();
    if (killed) throw new Error("provider killed");
    const relay = sd.addChain({ chainSpec: specs.relay, disableJsonRpc: true });
    chains.push(relay);
    const relayChain = await relay;
    if (killed) throw new Error("provider killed");
    const para = sd.addChain({
      chainSpec: specs.para,
      potentialRelayChains: [relayChain],
    });
    chains.push(para);
    return para;
  });

  const kill = () => {
    killed = true;
    // Reverse order: para before relay. The provider's own teardown may have
    // already removed the para chain; the double-remove throws harmlessly.
    for (const chain of chains.splice(0).reverse()) {
      chain.then((c) => c.remove()).catch(() => {});
    }
  };
  return { provider, kill };
}

export async function connectToNetwork(
  networkId: NetworkId,
  endpointOverride?: string,
): Promise<void> {
  const networks = networksSubject.getValue();
  const baseNetwork = networks[networkId];
  if (!baseNetwork) {
    throw new Error(`Unknown network: ${networkId}`);
  }

  let network: Network = baseNetwork;
  if (endpointOverride) {
    network = { ...baseNetwork, endpoints: [endpointOverride] };
    if (networkId === "custom") {
      localStorage.setItem(STORAGE_KEY_CUSTOM_URL, endpointOverride);
    }
  } else if (networkId === "custom") {
    const saved = localStorage.getItem(STORAGE_KEY_CUSTOM_URL);
    if (saved) network = { ...baseNetwork, endpoints: [saved] };
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

  const useLightClient =
    resolveTransport(network) === "light-client" && !endpointOverride;

  localStorage.setItem(STORAGE_KEY_NETWORK, networkId);
  networkSubject.next(network);
  apiSubject.next(undefined);
  blockNumberSubject.next(undefined);
  chainInfoSubject.next({});
  sudoKeySubject.next(undefined);
  transportSubject.next(useLightClient ? "light-client" : "rpc");

  if (!useLightClient && network.endpoints.length === 0) {
    blockSubscription?.unsubscribe();
    blockSubscription = null;
    clientSubject.next(undefined);
    statusSubject.next("disconnected");
    errorSubject.next(undefined);
    if (networkId === "custom") return;
    statusSubject.next("error");
    errorSubject.next(`Network ${network.name} has no endpoints available`);
    return;
  }

  statusSubject.next("connecting");
  errorSubject.next(undefined);

  try {
    let provider;

    if (useLightClient && network.chainSpec) {
      // Resolve specs here rather than inside the provider factory: errors
      // thrown there are swallowed and would leave the UI stuck on "connecting".
      const [para, relay] = await Promise.all([
        network.chainSpec.para(),
        network.chainSpec.relay(),
      ]);
      const killable = createSmoldotProvider({ para, relay });
      provider = killable.provider;
      killCurrentProvider = killable.kill;
    } else {
      const killable = createKillableWsProvider(network.endpoints[0]!);
      provider = killable.provider;
      killCurrentProvider = killable.kill;
    }

    const client = createClient(provider);
    clientSubject.next(client);

    const api = client.getTypedApi(network.descriptor) as TypedApi<typeof bulletin_paseo_next_v2>;
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

    // Subscribe to best block (clean up previous subscription first)
    blockSubscription?.unsubscribe();
    blockSubscription = client.bestBlocks$.subscribe({
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
  blockSubscription?.unsubscribe();
  blockSubscription = null;
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
  networksSubject,
  networkSubject,
  statusSubject,
  errorSubject,
  clientSubject,
  apiSubject,
  transportSubject,
  blockNumberSubject,
  chainInfoSubject,
]).pipe(
  map(([networks, network, status, error, client, api, transport, blockNumber, chainInfo]) => ({
    networks,
    network,
    status,
    error,
    client,
    api,
    transport,
    blockNumber,
    ...chainInfo,
  })),
  shareReplay(1)
);

// React hooks
export const [useChainState] = bind(chainState$, {
  networks: BULLETIN_NETWORKS,
  network: initialNetwork,
  status: "disconnected" as const,
  error: undefined,
  client: undefined,
  api: undefined,
  transport: resolveTransport(initialNetwork),
  blockNumber: undefined,
  chainName: undefined,
  specVersion: undefined,
  tokenSymbol: undefined,
  tokenDecimals: undefined,
  ss58Format: undefined,
});

export const [useNetwork] = bind(networkSubject);
export const [useConnectionStatus] = bind(statusSubject, "disconnected");
export const [useTransport] = bind(transportSubject, resolveTransport(initialNetwork));
export const [useBlockNumber] = bind(blockNumberSubject, undefined);
export const [useApi] = bind(apiSubject, undefined);
export const [useClient] = bind(clientSubject, undefined);
export const [useSudoKey] = bind(sudoKeySubject, undefined);

/**
 * Hook that returns a factory for creating AsyncBulletinClient instances.
 * Returns undefined if not connected. Call with a signer to get a client.
 *
 * @example
 * ```tsx
 * const createBulletinClient = useCreateBulletinClient();
 * // later in a handler:
 * const bulletinClient = createBulletinClient?.(signer);
 * ```
 */
export function useCreateBulletinClient(): ((signer: PolkadotSigner) => AsyncBulletinClient) | undefined {
  const api = useApi();
  const client = useClient();
  if (!api || !client) return undefined;
  return (signer: PolkadotSigner) => new AsyncBulletinClient(api, signer, client.submit);
}

// Direct access to subjects for non-React code
export const network$ = networkSubject.asObservable();
export const status$ = statusSubject.asObservable();
export const api$ = apiSubject.asObservable();
export const client$ = clientSubject.asObservable();
