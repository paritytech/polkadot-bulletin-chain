import { useState, useEffect, useRef } from "react";
import { useSearchParams } from "react-router-dom";
import {
  Download as DownloadIcon,
  Search,
  Copy,
  Check,
  AlertCircle,
  File,
  Wifi,
  WifiOff,
  Loader2,
  Globe,
  History,
} from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/Tabs";
import { CidInput } from "@/components/CidInput";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select";
import { formatBytes, bytesToHex, estimateBlockDate, formatBlockDuration, formatBlockNumber } from "@/utils/format";
import { CID, CidCodec, HashAlgorithm, parseCid } from "@parity/bulletin-sdk";
import * as digest from "multiformats/hashes/digest";
import { HeliaClient, type ConnectionInfo } from "@/lib/helia";
import { IPFS_GATEWAYS, PREFERRED_DOWNLOAD_METHOD, buildIpfsUrl, fetchFromIpfs } from "@/lib/ipfs";
import { useNetwork, useBlockNumber, useApi } from "@/state/chain.state";
import { useStorageHistory } from "@/state/history.state";
import { lookupCidOnChain, type OnChainTransaction } from "@/lib/cid-lookup";

const P2P_MULTIADDRS: Record<string, string> = {
  local: "/ip4/127.0.0.1/tcp/30334/ws/p2p/12D3KooWBmAwcd4PJNJvfV89HwE48nwkRmAgo8Vy3uQEyNNHBox2",
  westend: [
    "/dns4/westend-bulletin-collator-node-0.parity-testnet.parity.io/tcp/443/wss/p2p/12D3KooWSxYQRoTT9rZNZRrjCfG2fPpBwPumkQsxLroTKjX6Mvkw",
    "/dns4/westend-bulletin-collator-node-1.parity-testnet.parity.io/tcp/443/wss/p2p/12D3KooWSD5tovFkmja9aFYA6QM8eU3mFhZKdAuCsa5MgSsNDmxc",
    "/dns4/westend-bulletin-rpc-node-0.polkadot.io/tcp/443/wss/p2p/12D3KooWGb3sdXpdQPvL1wwHYHpQpMAEWxpgNNb6sndHmCByMXZw",
    "/dns4/westend-bulletin-rpc-node-1.polkadot.io/tcp/443/wss/p2p/12D3KooWN8hBVUWXNiur1w6EiEPkTJibbzpagZmm4cphMxWLv9yc",
  ].join("\n"),
  paseo: [
    "/dns4/paseo-bulletin-collator-node-0.parity-testnet.parity.io/tcp/443/wss/p2p/12D3KooWRuKisocQ2Z5hBZagV5YGxJMYuW13xT42sUiUCWf5bRtu",
    "/dns4/paseo-bulletin-collator-node-1.parity-testnet.parity.io/tcp/443/wss/p2p/12D3KooWSgdX2egCUiXtDUNV6hGh6JrtTb9vQ6iRfFMdnTemQDDp",
    "/dns4/paseo-bulletin-rpc-node-0.polkadot.io/tcp/443/wss/p2p/12D3KooWG7dt8yAMBaNrWh5juvHMGvJtPKTCaS87kkadWZKpV7ox",
    "/dns4/paseo-bulletin-rpc-node-1.polkadot.io/tcp/443/wss/p2p/12D3KooWSS9QNRiLGBoZrDrtXvPyBV7QrV7F3A1V8f6xAXECSnj5",
  ].join("\n"),
  "paseo-next-v2": [
    "/dns4/paseo-bulletin-next-collator-node-0.parity-testnet.parity.io/tcp/443/wss/p2p/12D3KooWDGdPBWpytPdNAXDT2KJWwmPXkxvxyQLGc7pRdFWeZnyB",
    "/dns4/paseo-bulletin-next-collator-node-1.parity-testnet.parity.io/tcp/443/wss/p2p/12D3KooWC45NgktSLMPQafAhi8TMAtiiatnmNc3Qv6wA74u7YBVc",
    "/dns4/paseo-bulletin-next-rpc-node-0.polkadot.io/tcp/443/wss/p2p/12D3KooWS4ptBbHGritdb1T7JPxKT2EN7FXvqq9rUp12jUvjnqQ1",
    "/dns4/paseo-bulletin-next-rpc-node-1.polkadot.io/tcp/443/wss/p2p/12D3KooWKMc4jJsU7fdEsis4AsM8Assk5jFqhEUEa2ZSiWJGKpfv",
  ].join("\n"),
  summit: [
    "/dns4/summit-bulletin-collator-node-0.parity-chains.parity.io/tcp/443/wss/p2p/12D3KooWC6q8q3NXscVcpxMbteYrmzjpy7NvYnD4QDRkAQJ9ng8r",
    "/dns4/summit-bulletin-collator-node-1.parity-chains.parity.io/tcp/443/wss/p2p/12D3KooWRiRRk8EzmENBD6SkP7v2riWa6s74X7wzhnx84SxfD4yr",
    "/dns4/summit-bulletin-rpc-node-0.parity-chains.parity.io/tcp/443/wss/p2p/12D3KooWSCrFvEXpRn9J5VC7TiabNwofVfbg3QPzJK9R5ZoDGjVq",
    "/dns4/summit-bulletin-rpc-node-1.parity-chains.parity.io/tcp/443/wss/p2p/12D3KooWHV6qNxpwkbTezwgsDW1xBL4J56o3xZnJXvRzHLdsMQJG",
  ].join("\n"),
};

interface FetchResult {
  cid: string;
  data: Uint8Array;
  size: number;
  isJSON: boolean;
  parsedJSON?: unknown;
}

type ConnectionStatus = "disconnected" | "connecting" | "connected" | "error";

function getDefaultMultiaddrs(networkId: string): string {
  return P2P_MULTIADDRS[networkId] ?? "";
}

function OnChainStatusContent({
  parsedCid,
  cidLookupLoading,
  cidLookupDone,
  cidLookup,
  currentBlock,
  retentionPeriod,
}: {
  parsedCid: CID | undefined;
  cidLookupLoading: boolean;
  cidLookupDone: boolean;
  cidLookup: OnChainTransaction | null;
  currentBlock: number | undefined;
  retentionPeriod: number | null;
}) {
  if (!parsedCid) {
    return <p className="text-sm text-muted-foreground">Enter a valid CID to check on-chain status</p>;
  }

  if (cidLookupLoading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        Searching on-chain...
      </div>
    );
  }

  if (cidLookupDone && !cidLookup) {
    return (
      <div className="space-y-2">
        <div className="flex items-start gap-2 text-sm text-amber-600 bg-amber-500/10 p-3 rounded-md">
          <AlertCircle className="h-4 w-4 mt-0.5 shrink-0" />
          <span>
            Not found on-chain. The data may have expired and been removed, or was never stored on this network.
          </span>
        </div>
      </div>
    );
  }

  if (!cidLookup || currentBlock === undefined || retentionPeriod === null) {
    return null;
  }

  const expiresAtBlock = cidLookup.blockNumber + retentionPeriod;
  const blocksRemaining = expiresAtBlock - currentBlock;
  const isExpired = blocksRemaining <= 0;
  const isExpiringSoon = !isExpired && blocksRemaining < 14400; // ~1 day

  return (
    <div className="space-y-3 text-sm">
      <div className="flex justify-between">
        <span className="text-muted-foreground">Stored at block</span>
        <span className="font-mono">{formatBlockNumber(cidLookup.blockNumber)} (idx {cidLookup.index})</span>
      </div>
      <div className="flex justify-between">
        <span className="text-muted-foreground">Upload date</span>
        <span>{estimateBlockDate(cidLookup.blockNumber, currentBlock).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}</span>
      </div>
      {cidLookup.hashing && (
        <div className="flex justify-between">
          <span className="text-muted-foreground">Hashing / Codec</span>
          <span className="font-mono text-xs">
            {cidLookup.hashing}
            {cidLookup.cidCodec !== undefined ? ` / 0x${cidLookup.cidCodec.toString(16)}` : ""}
          </span>
        </div>
      )}
      <div className="flex justify-between">
        <span className="text-muted-foreground">Expires at block</span>
        <span className="font-mono">{formatBlockNumber(expiresAtBlock)}</span>
      </div>
      <div className="flex justify-between items-center">
        <span className="text-muted-foreground">Retention</span>
        {isExpired ? (
          <Badge variant="destructive">Expired</Badge>
        ) : isExpiringSoon ? (
          <Badge className="bg-amber-500/10 text-amber-600 border-amber-500/20">
            {formatBlockDuration(blocksRemaining)} left
          </Badge>
        ) : (
          <Badge variant="secondary" className="bg-green-500/10 text-green-600">
            {formatBlockDuration(blocksRemaining)} left
          </Badge>
        )}
      </div>
      {isExpired && (
        <div className="flex items-start gap-2 text-sm text-destructive bg-destructive/10 p-3 rounded-md mt-2">
          <AlertCircle className="h-4 w-4 mt-0.5 shrink-0" />
          <span>
            This data has expired and may no longer be accessible. Consider re-uploading if needed.
          </span>
        </div>
      )}
    </div>
  );
}

export function Download() {
  const [searchParams, setSearchParams] = useSearchParams();
  const network = useNetwork();
  const blockNumber = useBlockNumber();
  const api = useApi();
  const storageHistory = useStorageHistory();

  // Filter history for current network
  const networkHistory = storageHistory.filter((entry) => entry.networkId === network.id);

  const [cidInput, setCidInput] = useState(searchParams.get("cid") || "");
  const [isCidValid, setIsCidValid] = useState(false);
  const [parsedCid, setParsedCid] = useState<CID | undefined>();

  const [cidInputMode, setCidInputMode] = useState<"cid" | "content-hash">("cid");
  const [contentHashInput, setContentHashInput] = useState("");
  const [hashAlgo, setHashAlgo] = useState<HashAlgorithm>(HashAlgorithm.Blake2b256);
  const [cidCodec, setCidCodec] = useState<CidCodec>(CidCodec.Raw);
  const [contentHashError, setContentHashError] = useState<string | null>(null);

  const [peerMultiaddrs, setPeerMultiaddrs] = useState(() => getDefaultMultiaddrs(network.id));
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>("disconnected");
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [connectedPeers, setConnectedPeers] = useState<ConnectionInfo[]>([]);
  const [localPeerId, setLocalPeerId] = useState<string | null>(null);

  const [isFetching, setIsFetching] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [fetchResult, setFetchResult] = useState<FetchResult | null>(null);

  const [copied, setCopied] = useState(false);
  const [displayMode, setDisplayMode] = useState<"text" | "hex" | "preview">("text");

  // On-chain CID lookup
  const [cidLookup, setCidLookup] = useState<OnChainTransaction | null>(null);
  const [cidLookupLoading, setCidLookupLoading] = useState(false);
  const [cidLookupDone, setCidLookupDone] = useState(false);
  const [retentionPeriod, setRetentionPeriod] = useState<number | null>(null);

  const [gatewayUrl, setGatewayUrl] = useState(
    () => IPFS_GATEWAYS[network.id] ?? ""
  );

  const activeTab = searchParams.get("tab") || PREFERRED_DOWNLOAD_METHOD[network.id] || "p2p";

  const heliaClientRef = useRef<HeliaClient | null>(null);
  const prevNetworkId = useRef(network.id);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      heliaClientRef.current?.stop();
    };
  }, []);

  // Reset everything when network actually changes (skip on mount so we
  // preserve the CID from URL query params; works with StrictMode)
  useEffect(() => {
    if (prevNetworkId.current === network.id) {
      return;
    }
    prevNetworkId.current = network.id;

    heliaClientRef.current?.stop();
    heliaClientRef.current = null;

    setPeerMultiaddrs(getDefaultMultiaddrs(network.id));
    setConnectionStatus("disconnected");
    setConnectionError(null);
    setConnectedPeers([]);
    setLocalPeerId(null);

    setCidInput("");
    setIsCidValid(false);
    setParsedCid(undefined);
    setIsFetching(false);
    setFetchError(null);
    setFetchResult(null);

    setGatewayUrl(IPFS_GATEWAYS[network.id] ?? "");

    // Clear tab param so the new network's preferred method takes effect
    setSearchParams((prev) => {
      const next = new URLSearchParams(prev);
      next.delete("tab");
      next.delete("cid");
      return next;
    });
  }, [network.id, setSearchParams]);

  // Update URL when CID changes
  useEffect(() => {
    if (cidInput) {
      setSearchParams((prev) => {
        const next = new URLSearchParams(prev);
        next.set("cid", cidInput);
        return next;
      });
    } else {
      setSearchParams((prev) => {
        const next = new URLSearchParams(prev);
        next.delete("cid");
        return next;
      });
    }
  }, [cidInput, setSearchParams]);

  // Fetch retention period once per api change
  useEffect(() => {
    if (!api) return;
    let cancelled = false;
    api.query.TransactionStorage.RetentionPeriod.getValue().then((period: bigint | number) => {
      if (!cancelled) setRetentionPeriod(Number(period));
    });
    return () => { cancelled = true; };
  }, [api]);

  // Look up CID on-chain when a valid CID is entered
  useEffect(() => {
    if (!parsedCid || !api) {
      setCidLookup(null);
      setCidLookupDone(false);
      return;
    }

    let cancelled = false;
    setCidLookupLoading(true);
    setCidLookupDone(false);
    setCidLookup(null);

    lookupCidOnChain(api, parsedCid).then((result) => {
      if (!cancelled) {
        setCidLookup(result);
        setCidLookupLoading(false);
        setCidLookupDone(true);
      }
    }).catch((err) => {
      if (!cancelled) {
        console.error("Failed to look up CID on chain:", err);
        setCidLookup(null);
        setCidLookupLoading(false);
        setCidLookupDone(true);
      }
    });

    return () => { cancelled = true; };
  }, [parsedCid?.toString(), api]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleCidChange = (value: string, isValid: boolean, cid?: CID) => {
    setCidInput(value);
    setIsCidValid(isValid);
    setParsedCid(cid);
    setFetchResult(null);
    setFetchError(null);
  };

  useEffect(() => {
    if (cidInputMode !== "content-hash") return;

    if (!contentHashInput.trim()) {
      setContentHashError(null);
      setCidInput("");
      setIsCidValid(false);
      setParsedCid(undefined);
      setFetchResult(null);
      setFetchError(null);
      return;
    }

    try {
      const cleaned = contentHashInput.trim().replace(/^0x/i, "");
      if (!/^[0-9a-fA-F]+$/.test(cleaned) || cleaned.length % 2 !== 0) {
        throw new Error("Content hash must be hex");
      }
      const bytes = new Uint8Array(cleaned.length / 2);
      for (let i = 0; i < bytes.length; i++) {
        bytes[i] = parseInt(cleaned.slice(i * 2, i * 2 + 2), 16);
      }
      if (bytes.length !== 32) {
        throw new Error("Content hash must be 32 bytes");
      }
      // multiformats `digest.create` types `digest` as `Uint8Array<ArrayBufferLike>`
      // but `CID.createV1` wants `Uint8Array<ArrayBuffer>`; cast through `any`
      // at this single seam rather than rewriting the call site.
      const mh = digest.create(hashAlgo, bytes) as any;
      const cid = CID.createV1(cidCodec, mh);
      setContentHashError(null);
      setCidInput(cid.toString());
      setIsCidValid(true);
      setParsedCid(cid);
      setFetchResult(null);
      setFetchError(null);
    } catch (e) {
      setContentHashError(e instanceof Error ? e.message : String(e));
      setCidInput("");
      setIsCidValid(false);
      setParsedCid(undefined);
    }
  }, [cidInputMode, contentHashInput, hashAlgo, cidCodec]);

  const handleHistorySelect = (cid: string) => {
    if (cid === "none") return;
    setCidInput(cid);
    // Try to parse it
    try {
      const parsed = parseCid(cid);
      setIsCidValid(true);
      setParsedCid(parsed);
    } catch {
      setIsCidValid(false);
      setParsedCid(undefined);
    }
    setFetchResult(null);
    setFetchError(null);
  };

  const handleConnect = async () => {
    // Parse multiaddrs (one per line or comma-separated)
    const addrs = peerMultiaddrs
      .split(/[\n,]/)
      .map((s) => s.trim())
      .filter((s) => s.length > 0);

    if (addrs.length === 0) {
      setConnectionError("Please enter at least one peer multiaddr");
      return;
    }

    setConnectionStatus("connecting");
    setConnectionError(null);
    setConnectedPeers([]);
    setLocalPeerId(null);

    // Stop existing client if any
    if (heliaClientRef.current) {
      await heliaClientRef.current.stop();
    }

    try {
      const client = new HeliaClient({
        peerMultiaddrs: addrs,
        onLog: (level, message, data) => {
          const prefix = { info: "INFO", debug: "DEBUG", error: "ERROR", success: "OK" }[level];
          console.log(`[Helia ${prefix}] ${message}`, data ?? "");
        },
      });

      const { peerId, connections } = await client.initialize();

      if (connections.length === 0) {
        throw new Error("Failed to connect to any peers");
      }

      heliaClientRef.current = client;
      setLocalPeerId(peerId);
      setConnectedPeers(connections);
      setConnectionStatus("connected");
    } catch (error) {
      console.error("Connection failed:", error);
      setConnectionError(error instanceof Error ? error.message : "Failed to connect");
      setConnectionStatus("error");
    }
  };

  const handleDisconnect = async () => {
    if (heliaClientRef.current) {
      await heliaClientRef.current.stop();
      heliaClientRef.current = null;
    }
    setConnectionStatus("disconnected");
    setConnectedPeers([]);
    setLocalPeerId(null);
    setFetchResult(null);
  };

  const handleFetch = async () => {
    if (!isCidValid || !parsedCid) return;

    setIsFetching(true);
    setFetchError(null);
    setFetchResult(null);

    try {
      let data: Uint8Array;
      let isJSON = false;
      let parsedJSON: unknown;

      if (activeTab === "gateway" && hasGateway) {
        const result = await fetchFromIpfs(parsedCid.toString(), gatewayUrl.trim());
        data = result.data;
        const contentType = result.contentType ?? "";
        if (contentType.includes("json")) {
          isJSON = true;
          try {
            parsedJSON = JSON.parse(new TextDecoder().decode(data));
          } catch {
            // not valid JSON despite content-type
          }
        }
      } else if (heliaClientRef.current) {
        const result = await heliaClientRef.current.fetchData(parsedCid);
        data = result.data;
        isJSON = result.isJSON;
        parsedJSON = result.parsedJSON;
      } else {
        return;
      }

      setFetchResult({
        cid: parsedCid.toString(),
        data,
        size: data.length,
        isJSON,
        parsedJSON,
      });
    } catch (err) {
      console.error("Fetch failed:", err);
      setFetchError(err instanceof Error ? err.message : "Failed to fetch data");
    } finally {
      setIsFetching(false);
    }
  };

  const copyToClipboard = async (text: string) => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const downloadData = () => {
    if (!fetchResult) return;

    const buffer = new ArrayBuffer(fetchResult.data.length);
    new Uint8Array(buffer).set(fetchResult.data);
    const blob = new Blob([buffer], { type: "application/octet-stream" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${fetchResult.cid.slice(0, 16)}...`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  const renderContent = () => {
    if (!fetchResult) return null;

    const { data, isJSON, parsedJSON } = fetchResult;

    // Text view (with JSON formatting if applicable)
    if (displayMode === "text") {
      if (isJSON && parsedJSON) {
        return (
          <pre className="bg-secondary p-4 rounded-md overflow-auto max-h-[400px] text-sm font-mono whitespace-pre-wrap">
            {JSON.stringify(parsedJSON, null, 2)}
          </pre>
        );
      }
      try {
        const text = new TextDecoder().decode(data);
        return (
          <pre className="bg-secondary p-4 rounded-md overflow-auto max-h-[400px] text-sm font-mono whitespace-pre-wrap">
            {text}
          </pre>
        );
      } catch {
        return <p className="text-muted-foreground">Unable to decode as text. Try hex view.</p>;
      }
    }

    // Hex view
    if (displayMode === "hex") {
      const hex = bytesToHex(data.slice(0, 1000));
      const truncated = data.length > 1000;
      return (
        <div>
          <pre className="bg-secondary p-4 rounded-md overflow-auto max-h-[400px] text-sm font-mono break-all">
            {hex}
          </pre>
          {truncated && (
            <p className="text-sm text-muted-foreground mt-2">
              Showing first 1000 bytes of {formatBytes(data.length)}
            </p>
          )}
        </div>
      );
    }

    // Preview (for images)
    if (displayMode === "preview") {
      const buffer = new ArrayBuffer(data.length);
      new Uint8Array(buffer).set(data);
      const blob = new Blob([buffer]);
      const url = URL.createObjectURL(blob);
      return (
        <img
          src={url}
          alt="Content preview"
          className="max-w-full max-h-[400px] rounded-md"
          onLoad={() => URL.revokeObjectURL(url)}
          onError={() => URL.revokeObjectURL(url)}
        />
      );
    }

    return null;
  };

  const isConnected = connectionStatus === "connected";
  const hasGateway = gatewayUrl.trim().length > 0;
  const canFetch = activeTab === "gateway" ? hasGateway : isConnected;

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Download Data</h1>
        <p className="text-muted-foreground">Retrieve data from the Bulletin Chain via P2P or IPFS Gateway</p>
      </div>

      <Tabs
        value={activeTab}
        onValueChange={(v) => {
          setFetchError(null);
          setFetchResult(null);
          setSearchParams((prev) => {
            const next = new URLSearchParams(prev);
            next.set("tab", v);
            return next;
          });
        }}
      >
        <TabsList>
          <TabsTrigger value="p2p">
            <Wifi className="h-4 w-4 mr-2" />
            P2P Connection
          </TabsTrigger>
          <TabsTrigger value="gateway">
            <Globe className="h-4 w-4 mr-2" />
            IPFS Gateway
          </TabsTrigger>
        </TabsList>

        {/* Tab 1: P2P Connection */}
        <TabsContent value="p2p" className="mt-4">
          <div className="grid gap-6 lg:grid-cols-3">
            {/* Connection Card */}
            <div className="lg:col-span-2">
              <Card>
                <CardHeader>
                  <CardTitle className="flex items-center gap-2">
                    {isConnected ? (
                      <Wifi className="h-5 w-5 text-green-500" />
                    ) : (
                      <WifiOff className="h-5 w-5 text-muted-foreground" />
                    )}
                    P2P Connection
                  </CardTitle>
                  <CardDescription>
                    Connect to bulletin-chain validator nodes via WebSocket using <strong>Helia</strong> (IPFS in the browser)
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div className="space-y-2">
                    <label className="text-sm font-medium">Peer Multiaddrs</label>
                    <textarea
                      value={peerMultiaddrs}
                      onChange={(e) => setPeerMultiaddrs(e.target.value)}
                      placeholder="/ip4/127.0.0.1/tcp/30334/ws/p2p/<peer-id>"
                      data-testid="peer-multiaddrs"
                      disabled={connectionStatus === "connecting" || isConnected}
                      className="flex min-h-[80px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 font-mono"
                      rows={3}
                    />
                    <p className="text-xs text-muted-foreground">
                      Enter one multiaddr per line. Get this from your validator node logs.
                    </p>
                  </div>

                  {connectionError && (
                    <div className="flex items-start gap-2 text-destructive text-sm">
                      <AlertCircle className="h-4 w-4 mt-0.5" />
                      <span>{connectionError}</span>
                    </div>
                  )}

                  {isConnected && (
                    <div className="space-y-2 text-sm">
                      <div className="flex items-center gap-2">
                        <Badge variant="secondary" className="bg-green-500/10 text-green-600">
                          Connected
                        </Badge>
                        <span className="text-muted-foreground">
                          {connectedPeers.length} peer{connectedPeers.length !== 1 ? "s" : ""}
                        </span>
                      </div>
                      {localPeerId && (
                        <p className="text-xs text-muted-foreground font-mono truncate">
                          Local: {localPeerId}
                        </p>
                      )}
                    </div>
                  )}

                  <div className="flex gap-2">
                    {!isConnected ? (
                      <Button
                        onClick={handleConnect}
                        disabled={connectionStatus === "connecting"}
                        className="flex-1"
                      >
                        {connectionStatus === "connecting" ? (
                          <>
                            <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                            Connecting...
                          </>
                        ) : (
                          <>
                            <Wifi className="h-4 w-4 mr-2" />
                            Connect
                          </>
                        )}
                      </Button>
                    ) : (
                      <Button onClick={handleDisconnect} variant="outline" className="flex-1">
                        <WifiOff className="h-4 w-4 mr-2" />
                        Disconnect
                      </Button>
                    )}
                  </div>

                  <div className="border-t pt-4 text-sm text-muted-foreground space-y-2">
                    <p>
                      Get the peer multiaddr from your Bulletin Chain node logs. It looks like:
                      <code className="block mt-1 text-xs bg-secondary p-1 rounded">
                        /ip4/.../tcp/.../ws/p2p/12D3KooW...
                      </code>
                    </p>
                    <p>Make sure your node has the WebSocket transport enabled (default port 30334).</p>
                    <p>Data is fetched directly via P2P using the Bitswap protocol.</p>
                  </div>
                </CardContent>
              </Card>
            </div>

            {/* Connected Peers Card */}
            <div>
              <Card>
                <CardHeader>
                  <CardTitle>Connected Peers</CardTitle>
                  <CardDescription>Active P2P connections</CardDescription>
                </CardHeader>
                <CardContent>
                  {isConnected && connectedPeers.length > 0 ? (
                    <div className="space-y-3">
                      {connectedPeers.map((peer, i) => (
                        <div key={i} className="text-sm space-y-1">
                          <div className="flex items-center gap-2">
                            <div className="w-2 h-2 bg-green-500 rounded-full" />
                            <span className="font-mono text-xs truncate">{peer.peerId.slice(0, 20)}...</span>
                          </div>
                          <p className="text-xs text-muted-foreground pl-4">{peer.direction}</p>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <p className="text-sm text-muted-foreground">No peers connected</p>
                  )}
                </CardContent>
              </Card>
            </div>
          </div>
        </TabsContent>

        {/* Tab 2: IPFS Gateway */}
        <TabsContent value="gateway" className="mt-4">
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Globe className="h-5 w-5" />
                IPFS Gateway
              </CardTitle>
              <CardDescription>
                Access Bulletin Chain data through an HTTP IPFS gateway
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="space-y-2">
                <label className="text-sm font-medium">Gateway URL</label>
                <input
                  type="text"
                  value={gatewayUrl}
                  onChange={(e) => setGatewayUrl(e.target.value)}
                  placeholder="https://ipfs.example.com"
                  data-testid="gateway-url-input"
                  className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 font-mono"
                />
                <p className="text-xs text-muted-foreground">
                  The gateway URL used to fetch data. The <code>/ipfs/&lt;cid&gt;</code> path is appended automatically.
                </p>
              </div>

              {cidInput && isCidValid ? (
                <div className="space-y-2">
                  <label className="text-sm font-medium">Gateway Link</label>
                  <div className="flex items-center gap-2">
                    <code className="flex-1 text-xs bg-secondary p-2 rounded-md break-all">
                      {buildIpfsUrl(parsedCid!.toString(), gatewayUrl)}
                    </code>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => copyToClipboard(buildIpfsUrl(parsedCid!.toString(), gatewayUrl))}
                    >
                      {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                    </Button>
                  </div>
                  <a
                    href={buildIpfsUrl(parsedCid!.toString(), gatewayUrl)}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center gap-2 text-sm text-primary hover:underline"
                  >
                    <Globe className="h-4 w-4" />
                    Open in browser
                  </a>
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">
                  Enter a valid CID below to generate a gateway link.
                </p>
              )}
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>

      {/* Always visible: Fetch by CID + CID Info */}
      <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2 space-y-6">
          {/* Search Card */}
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Search className="h-5 w-5" />
                Fetch by CID
              </CardTitle>
              <CardDescription>
                {canFetch
                  ? activeTab === "gateway"
                    ? "Enter a CID to retrieve data via IPFS Gateway"
                    : "Enter a CID to retrieve data via P2P"
                  : "Enter a CID to retrieve data"}
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              {!canFetch && (
                <div className="flex items-start gap-2 text-sm text-amber-600 bg-amber-500/10 p-3 rounded-md">
                  <AlertCircle className="h-4 w-4 mt-0.5 shrink-0" />
                  <span>
                    No data source configured. Connect to a peer in the{" "}
                    <strong>P2P Connection</strong> tab or set a gateway URL in the{" "}
                    <strong>IPFS Gateway</strong> tab.
                  </span>
                </div>
              )}
              <Tabs
                value={cidInputMode}
                onValueChange={(v) => {
                  setCidInputMode(v as "cid" | "content-hash");
                  setFetchError(null);
                  setFetchResult(null);
                  setContentHashError(null);
                }}
              >
                <TabsList>
                  <TabsTrigger value="cid">By CID</TabsTrigger>
                  <TabsTrigger value="content-hash">By ContentHash</TabsTrigger>
                </TabsList>

                <TabsContent value="cid" className="mt-4 space-y-2">
                  <label className="text-sm font-medium">CID</label>
                  <CidInput
                    value={cidInput}
                    onChange={handleCidChange}
                    disabled={isFetching || !canFetch}
                  />
                </TabsContent>

                <TabsContent value="content-hash" className="mt-4 space-y-4">
                  <div className="grid sm:grid-cols-2 gap-4">
                    <div className="space-y-2">
                      <label className="text-sm font-medium">Hashing algorithm</label>
                      <Select
                        value={String(hashAlgo)}
                        onValueChange={(v) => setHashAlgo(Number(v) as HashAlgorithm)}
                        disabled={isFetching || !canFetch}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value={String(HashAlgorithm.Blake2b256)}>Blake2b-256</SelectItem>
                          <SelectItem value={String(HashAlgorithm.Sha2_256)}>SHA2-256</SelectItem>
                          <SelectItem value={String(HashAlgorithm.Keccak256)}>Keccak-256</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="space-y-2">
                      <label className="text-sm font-medium">Codec</label>
                      <Select
                        value={String(cidCodec)}
                        onValueChange={(v) => setCidCodec(Number(v) as CidCodec)}
                        disabled={isFetching || !canFetch}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value={String(CidCodec.Raw)}>Raw (0x55)</SelectItem>
                          <SelectItem value={String(CidCodec.DagPb)}>DAG-PB (0x70)</SelectItem>
                          <SelectItem value={String(CidCodec.DagCbor)}>DAG-CBOR (0x71)</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                  </div>

                  <div className="space-y-2">
                    <label className="text-sm font-medium">Content hash</label>
                    <input
                      type="text"
                      value={contentHashInput}
                      onChange={(e) => setContentHashInput(e.target.value)}
                      placeholder="0x… (32-byte hex digest)"
                      disabled={isFetching || !canFetch}
                      className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 font-mono"
                    />
                    {contentHashError ? (
                      <p className="text-xs text-destructive">{contentHashError}</p>
                    ) : (
                      <p className="text-xs text-muted-foreground">
                        Pre-computed digest produced by the selected hashing algorithm.
                      </p>
                    )}
                  </div>

                  {isCidValid && cidInput && (
                    <div className="space-y-2 border-t pt-3">
                      <label className="text-sm font-medium">Computed CID</label>
                      <div className="flex items-center gap-2">
                        <code className="flex-1 text-xs bg-secondary p-2 rounded-md break-all">
                          {cidInput}
                        </code>
                        <Button variant="outline" size="sm" onClick={() => copyToClipboard(cidInput)}>
                          {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                        </Button>
                      </div>
                    </div>
                  )}
                </TabsContent>
              </Tabs>

              <Button
                onClick={handleFetch}
                disabled={!isCidValid || isFetching || !canFetch}
                className="w-full"
              >
                {isFetching ? (
                  <>
                    <Spinner size="sm" className="mr-2" />
                    {activeTab === "gateway" ? "Fetching via Gateway..." : "Fetching via P2P..."}
                  </>
                ) : (
                  <>
                    <DownloadIcon className="h-4 w-4 mr-2" />
                    Fetch Data
                  </>
                )}
              </Button>

              {activeTab === "gateway" && hasGateway && cidInput && isCidValid && (
                <div className="space-y-2 border-t pt-4">
                  <label className="text-sm font-medium">Gateway Link</label>
                  <div className="flex items-center gap-2">
                    <code className="flex-1 text-xs bg-secondary p-2 rounded-md break-all">
                      {buildIpfsUrl(parsedCid!.toString(), gatewayUrl)}
                    </code>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => copyToClipboard(buildIpfsUrl(parsedCid!.toString(), gatewayUrl))}
                    >
                      {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                    </Button>
                  </div>
                  <a
                    href={buildIpfsUrl(parsedCid!.toString(), gatewayUrl)}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center gap-2 text-sm text-primary hover:underline"
                  >
                    <Globe className="h-4 w-4" />
                    Open in browser
                  </a>
                </div>
              )}
            </CardContent>
          </Card>

          {/* Error Display */}
          {fetchError && (
            <Card className="border-destructive">
              <CardContent className="pt-6">
                <div className="flex items-start gap-3 text-destructive">
                  <AlertCircle className="h-5 w-5 mt-0.5" />
                  <div>
                    <p className="font-medium">Fetch Failed</p>
                    <p className="text-sm mt-1">{fetchError}</p>
                  </div>
                </div>
              </CardContent>
            </Card>
          )}

          {/* Result Display */}
          {fetchResult && (
            <Card>
              <CardHeader>
                <div className="flex items-center justify-between">
                  <div>
                    <CardTitle className="flex items-center gap-2">
                      <File className="h-5 w-5" />
                      Content
                    </CardTitle>
                    <CardDescription>
                      Retrieved {formatBytes(fetchResult.size)}
                      {fetchResult.isJSON && " (JSON)"}
                    </CardDescription>
                  </div>
                  <div className="flex gap-2">
                    <Button
                      variant={displayMode === "text" ? "secondary" : "ghost"}
                      size="sm"
                      onClick={() => setDisplayMode("text")}
                    >
                      Text
                    </Button>
                    <Button
                      variant={displayMode === "hex" ? "secondary" : "ghost"}
                      size="sm"
                      onClick={() => setDisplayMode("hex")}
                    >
                      Hex
                    </Button>
                    <Button
                      variant={displayMode === "preview" ? "secondary" : "ghost"}
                      size="sm"
                      onClick={() => setDisplayMode("preview")}
                    >
                      Preview
                    </Button>
                  </div>
                </div>
              </CardHeader>
              <CardContent className="space-y-4">
                {renderContent()}

                <div className="flex gap-2 pt-2">
                  <Button variant="outline" size="sm" onClick={downloadData}>
                    <DownloadIcon className="h-4 w-4 mr-2" />
                    Download
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => copyToClipboard(fetchResult.cid)}
                  >
                    {copied ? (
                      <Check className="h-4 w-4 mr-2" />
                    ) : (
                      <Copy className="h-4 w-4 mr-2" />
                    )}
                    Copy CID
                  </Button>
                </div>
              </CardContent>
            </Card>
          )}
        </div>

        {/* Sidebar: History + CID Info */}
        <div className="space-y-6">
          {/* My Storage History */}
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <History className="h-5 w-5" />
                My Storage
              </CardTitle>
              <CardDescription>Previously stored data on this network</CardDescription>
            </CardHeader>
            <CardContent>
              {networkHistory.length > 0 ? (
                <div className="space-y-3">
                  <Select onValueChange={handleHistorySelect}>
                    <SelectTrigger>
                      <SelectValue placeholder="Select from history..." />
                    </SelectTrigger>
                    <SelectContent>
                      {networkHistory.map((entry) => (
                        <SelectItem key={`${entry.blockNumber}-${entry.index}`} value={entry.cid}>
                          <div className="flex flex-col items-start">
                            <span className="text-xs font-medium">
                              {entry.label || `Block #${entry.blockNumber}`}
                            </span>
                            <span className="text-xs text-muted-foreground font-mono">
                              {entry.cid.slice(0, 20)}...
                            </span>
                          </div>
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <p className="text-xs text-muted-foreground">
                    {networkHistory.length} item{networkHistory.length !== 1 ? "s" : ""} in history
                  </p>
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">
                  No storage history yet. Upload data to see it here.
                </p>
              )}
            </CardContent>
          </Card>

          {/* CID Info */}
          <Card>
            <CardHeader>
              <CardTitle>CID Info</CardTitle>
              <CardDescription>Parsed CID details</CardDescription>
            </CardHeader>
            <CardContent>
              {parsedCid ? (
                <div className="space-y-3 text-sm">
                  <div>
                    <div className="flex items-center justify-between">
                      <span className="text-muted-foreground">CID</span>
                      <button
                        onClick={() => copyToClipboard(parsedCid.toString())}
                        className="text-muted-foreground hover:text-foreground transition-colors"
                        title="Copy CID"
                      >
                        {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                      </button>
                    </div>
                    <p className="font-mono text-xs mt-1 break-all">{parsedCid.toString()}</p>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Version</span>
                    <Badge variant="secondary">CIDv{parsedCid.version}</Badge>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Codec</span>
                    <span className="font-mono">0x{parsedCid.code.toString(16)}</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Hash</span>
                    <span className="font-mono">0x{parsedCid.multihash.code.toString(16)}</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Digest Size</span>
                    <span>{parsedCid.multihash.size} bytes</span>
                  </div>
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">Enter a valid CID to see details</p>
              )}
            </CardContent>
          </Card>

          {/* On-chain Status */}
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Search className="h-5 w-5" />
                On-chain Status
              </CardTitle>
              <CardDescription>Storage and retention info from the chain</CardDescription>
            </CardHeader>
            <CardContent>
              <OnChainStatusContent
                parsedCid={parsedCid}
                cidLookupLoading={cidLookupLoading}
                cidLookupDone={cidLookupDone}
                cidLookup={cidLookup}
                currentBlock={blockNumber}
                retentionPeriod={retentionPeriod}
              />
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
