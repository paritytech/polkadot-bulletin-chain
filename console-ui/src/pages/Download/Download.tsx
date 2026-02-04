import { useState, useEffect } from "react";
import { useSearchParams } from "react-router-dom";
import { Download as DownloadIcon, Search, ExternalLink, Copy, Check, AlertCircle, File } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import { CidInput } from "@/components/CidInput";
import { formatBytes, bytesToHex } from "@/utils/format";
import { CID } from "multiformats/cid";

const DEFAULT_IPFS_GATEWAY = "http://127.0.0.1:8283";

interface FetchResult {
  cid: string;
  data: Uint8Array;
  contentType?: string;
  size: number;
}

export function Download() {
  const [searchParams, setSearchParams] = useSearchParams();

  const [cidInput, setCidInput] = useState(searchParams.get("cid") || "");
  const [isCidValid, setIsCidValid] = useState(false);
  const [parsedCid, setParsedCid] = useState<CID | undefined>();

  const [ipfsGateway, setIpfsGateway] = useState(DEFAULT_IPFS_GATEWAY);

  const [isFetching, setIsFetching] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [fetchResult, setFetchResult] = useState<FetchResult | null>(null);

  const [copied, setCopied] = useState(false);
  const [displayMode, setDisplayMode] = useState<"text" | "hex" | "preview">("text");

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

  const handleFetch = async () => {
    if (!isCidValid || !cidInput) return;

    setIsFetching(true);
    setFetchError(null);
    setFetchResult(null);

    try {
      const url = `${ipfsGateway}/ipfs/${cidInput}`;
      const response = await fetch(url);

      if (!response.ok) {
        throw new Error(`HTTP ${response.status}: ${response.statusText}`);
      }

      const contentType = response.headers.get("content-type") || undefined;
      const arrayBuffer = await response.arrayBuffer();
      const data = new Uint8Array(arrayBuffer);

      setFetchResult({
        cid: cidInput,
        data,
        contentType,
        size: data.length,
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

    // Create a new ArrayBuffer copy for Blob compatibility
    const buffer = new ArrayBuffer(fetchResult.data.length);
    new Uint8Array(buffer).set(fetchResult.data);
    const blob = new Blob([buffer], {
      type: fetchResult.contentType || "application/octet-stream",
    });
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

    const { data, contentType } = fetchResult;

    // Check if it's an image
    if (contentType?.startsWith("image/") && displayMode === "preview") {
      const buffer = new ArrayBuffer(data.length);
      new Uint8Array(buffer).set(data);
      const blob = new Blob([buffer], { type: contentType });
      const url = URL.createObjectURL(blob);
      return (
        <img
          src={url}
          alt="Content preview"
          className="max-w-full max-h-[400px] rounded-md"
          onLoad={() => URL.revokeObjectURL(url)}
        />
      );
    }

    // Text view
    if (displayMode === "text") {
      try {
        const text = new TextDecoder().decode(data);
        return (
          <pre className="bg-secondary p-4 rounded-md overflow-auto max-h-[400px] text-sm font-mono whitespace-pre-wrap">
            {text}
          </pre>
        );
      } catch {
        return (
          <p className="text-muted-foreground">
            Unable to decode as text. Try hex view.
          </p>
        );
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

    return null;
  };

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Download Data</h1>
        <p className="text-muted-foreground">
          Retrieve data from the Bulletin Chain by CID
        </p>
      </div>

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
                Enter a CID to retrieve data from the IPFS gateway
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="space-y-2">
                <label className="text-sm font-medium">CID</label>
                <CidInput
                  value={cidInput}
                  onChange={handleCidChange}
                  disabled={isFetching}
                />
              </div>

              <div className="space-y-2">
                <label className="text-sm font-medium">IPFS Gateway</label>
                <Input
                  value={ipfsGateway}
                  onChange={(e) => setIpfsGateway(e.target.value)}
                  placeholder="http://127.0.0.1:8283"
                  disabled={isFetching}
                />
              </div>

              <Button
                onClick={handleFetch}
                disabled={!isCidValid || isFetching}
                className="w-full"
              >
                {isFetching ? (
                  <>
                    <Spinner size="sm" className="mr-2" />
                    Fetching...
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
                      Retrieved {formatBytes(fetchResult.size)}
                      {fetchResult.contentType && ` (${fetchResult.contentType})`}
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
                    {fetchResult.contentType?.startsWith("image/") && (
                      <Button
                        variant={displayMode === "preview" ? "secondary" : "ghost"}
                        size="sm"
                        onClick={() => setDisplayMode("preview")}
                      >
                        Preview
                      </Button>
                    )}
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
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => window.open(`${ipfsGateway}/ipfs/${fetchResult.cid}`, "_blank")}
                  >
                    <ExternalLink className="h-4 w-4 mr-2" />
                    Open Gateway
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
                    <span className="font-mono">
                      0x{parsedCid.multihash.code.toString(16)}
                    </span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Digest Size</span>
                    <span>{parsedCid.multihash.size} bytes</span>
                  </div>
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">
                  Enter a valid CID to see details
                </p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Tips</CardTitle>
            </CardHeader>
            <CardContent className="text-sm text-muted-foreground space-y-2">
              <p>
                Make sure your Bulletin Chain node is running with IPFS enabled
                (--ipfs-server flag).
              </p>
              <p>
                The default IPFS gateway is http://127.0.0.1:8283 for local nodes.
              </p>
              <p>
                For Westend/Polkadot, use the appropriate RPC endpoints.
              </p>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
