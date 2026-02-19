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
} from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import { CidInput } from "@/components/CidInput";
import { formatBytes, bytesToHex } from "@/utils/format";
import { CID } from "multiformats/cid";
import { HeliaClient, type ConnectionInfo } from "@/lib/helia";
import { useNetwork } from "@/state/chain.state";

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
  return P2P_MULTIADDRS[networkId] ?? P2P_MULTIADDRS["local"]!;
}

export function Download() {
  const [searchParams, setSearchParams] = useSearchParams();
  const network = useNetwork();

  const [cidInput, setCidInput] = useState(searchParams.get("cid") || "");
  const [isCidValid, setIsCidValid] = useState(false);
  const [parsedCid, setParsedCid] = useState<CID | undefined>();

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

  const heliaClientRef = useRef<HeliaClient | null>(null);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      heliaClientRef.current?.stop();
    };
  }, []);

  // Update default multiaddrs when network changes (only when not connected)
  useEffect(() => {
    if (connectionStatus === "disconnected" || connectionStatus === "error") {
      setPeerMultiaddrs(getDefaultMultiaddrs(network.id));
    }
  }, [network.id, connectionStatus]);

  // Update URL when CID changes
  useEffect(() => {
    if (cidInput) {
      setSearchParams({ cid: cidInput });
    } else {
      setSearchParams({});
    }
  }, [cidInput, setSearchParams]);

  const handleCidChange = (value: string, isValid: boolean, cid?: CID) => {
    setCidInput(value);
    setIsCidValid(isValid);
    setParsedCid(cid);
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
    if (!isCidValid || !parsedCid || !heliaClientRef.current) return;

    setIsFetching(true);
    setFetchError(null);
    setFetchResult(null);

    try {
      const result = await heliaClientRef.current.fetchData(parsedCid);

      setFetchResult({
        cid: parsedCid.toString(),
        data: result.data,
        size: result.data.length,
        isJSON: result.isJSON,
        parsedJSON: result.parsedJSON,
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

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Download Data</h1>
        <p className="text-muted-foreground">Retrieve data from the Bulletin Chain via P2P</p>
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2 space-y-6">
          {/* Connection Card */}
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
            </CardContent>
          </Card>

          {/* Search Card */}
          <Card className={!isConnected ? "opacity-50" : ""}>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Search className="h-5 w-5" />
                Fetch by CID
              </CardTitle>
              <CardDescription>
                {isConnected
                  ? "Enter a CID to retrieve data via P2P"
                  : "Connect to a peer first to fetch data"}
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="space-y-2">
                <label className="text-sm font-medium">CID</label>
                <CidInput
                  value={cidInput}
                  onChange={handleCidChange}
                  disabled={isFetching || !isConnected}
                />
              </div>

              <Button
                onClick={handleFetch}
                disabled={!isCidValid || isFetching || !isConnected}
                className="w-full"
              >
                {isFetching ? (
                  <>
                    <Spinner size="sm" className="mr-2" />
                    Fetching via P2P...
                  </>
                ) : (
                  <>
                    <DownloadIcon className="h-4 w-4 mr-2" />
                    Fetch Data
                  </>
                )}
              </Button>
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
                      Retrieved {formatBytes(fetchResult.size)} via P2P
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

        {/* Sidebar */}
        <div className="space-y-6">
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

          {/* Connected Peers Card */}
          {isConnected && connectedPeers.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle>Connected Peers</CardTitle>
                <CardDescription>Active P2P connections</CardDescription>
              </CardHeader>
              <CardContent>
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
              </CardContent>
            </Card>
          )}

          <Card>
            <CardHeader>
              <CardTitle>Tips</CardTitle>
            </CardHeader>
            <CardContent className="text-sm text-muted-foreground space-y-2">
              <p>
                Get the peer multiaddr from your Bulletin Chain node logs. It looks like:
                <code className="block mt-1 text-xs bg-secondary p-1 rounded">
                  /ip4/.../tcp/.../ws/p2p/12D3KooW...
                </code>
              </p>
              <p>Make sure your node has the WebSocket transport enabled (default port 30334).</p>
              <p>Data is fetched directly via P2P using the Bitswap protocol.</p>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
