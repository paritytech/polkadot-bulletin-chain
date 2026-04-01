import { useState, useCallback, useEffect, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { Upload as UploadIcon, Copy, Check, ExternalLink, AlertCircle, RefreshCw, Info, X, Shield } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Textarea } from "@/components/ui/Textarea";
import { Input } from "@/components/ui/Input";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/Tabs";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select";
import { FileUpload } from "@/components/FileUpload";
import { AuthorizationCard } from "@/components/AuthorizationCard";
import { useApi, useClient, useChainState, useCreateBulletinClient } from "@/state/chain.state";
import { useSelectedAccount } from "@/state/wallet.state";
import {
  useAuthorization,
  usePreimageAuth,
  usePreimageAuthLoading,
  checkPreimageAuthorization,
  clearPreimageAuth,
} from "@/state/storage.state";
import { addStorageEntry } from "@/state/history.state";
import { formatBytes } from "@/utils/format";
import { getContentHash, CidCodec, HashAlgorithm, WaitFor } from "@bulletin/sdk";
import { useProgressHandler } from "@/hooks/useProgressHandler";
import { bytesToHex } from "@/utils/format";
import { Binary } from "polkadot-api";

const HASH_ALGORITHMS: { value: HashAlgorithm; label: string }[] = [
  { value: HashAlgorithm.Blake2b256, label: "Blake2b-256 (default)" },
  { value: HashAlgorithm.Sha2_256, label: "SHA2-256" },
];

const CID_CODECS: { value: CidCodec; label: string }[] = [
  { value: CidCodec.Raw, label: "Raw (0x55)" },
  { value: CidCodec.DagPb, label: "DAG-PB (0x70)" },
];

interface UploadResult {
  cid: string;
  contentHash: string;
  blockHash?: string;
  blockNumber?: number;
  index?: number;
  size: number;
  unsigned?: boolean;
}

export function Upload() {
  const api = useApi();
  const client = useClient();
  const createBulletinClient = useCreateBulletinClient();
  const { network } = useChainState();
  const navigate = useNavigate();
  const selectedAccount = useSelectedAccount();
  const authorization = useAuthorization();
  const preimageAuth = usePreimageAuth();
  const preimageAuthLoading = usePreimageAuthLoading();
  const [txStatus, setTxStatus] = useState<string | null>(null);
  const handleProgress = useProgressHandler(setTxStatus);

  const [inputMode, setInputMode] = useState<"text" | "file">("text");
  const [textData, setTextData] = useState("");
  const [fileData, setFileData] = useState<Uint8Array | null>(null);
  const [fileName, setFileName] = useState<string | null>(null);

  const [hashAlgorithm, setHashAlgorithm] = useState<HashAlgorithm>(HashAlgorithm.Blake2b256);
  const [cidCodec, setCidCodec] = useState<CidCodec>(CidCodec.Raw);

  const [isUploading, setIsUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [uploadResult, setUploadResult] = useState<UploadResult | null>(null);
  const [copied, setCopied] = useState(false);

  const getData = useCallback((): Uint8Array | null => {
    if (inputMode === "text") {
      if (!textData.trim()) return null;
      return new TextEncoder().encode(textData);
    }
    return fileData;
  }, [inputMode, textData, fileData]);

  const dataSize = getData()?.length ?? 0;

  const debounceTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  // Check preimage authorization when data or hash algorithm changes
  useEffect(() => {
    if (debounceTimer.current) {
      clearTimeout(debounceTimer.current);
    }

    const data = getData();
    if (!data || !api) {
      clearPreimageAuth();
      return;
    }

    debounceTimer.current = setTimeout(async () => {
      try {
        const contentHash = await getContentHash(data, hashAlgorithm);
        await checkPreimageAuthorization(api, contentHash);
      } catch (err) {
        console.error("Failed to check preimage authorization:", err);
      }
    }, 300);

    return () => {
      if (debounceTimer.current) {
        clearTimeout(debounceTimer.current);
      }
    };
  }, [api, inputMode, textData, fileData, hashAlgorithm, getData]);

  const hasAccountAuth =
    selectedAccount?.polkadotSigner &&
    authorization &&
    authorization.bytes >= BigInt(dataSize) &&
    authorization.transactions > 0n;

  const hasPreimageAuth =
    preimageAuth &&
    preimageAuth.bytes >= BigInt(dataSize) &&
    preimageAuth.transactions > 0n;

  const canUpload =
    api &&
    client &&
    dataSize > 0 &&
    (hasAccountAuth || hasPreimageAuth);

  const handleFileSelect = useCallback((file: File | null, data: Uint8Array | null) => {
    setFileData(data);
    setFileName(file?.name ?? null);
    setUploadResult(null);
    setUploadError(null);
  }, []);

  // Preimage auth is preferred when available (same as pallet behavior)
  const willUseUnsigned = !!hasPreimageAuth;

  const handleUpload = async () => {
    if (!api || !client) return;

    const data = getData();
    if (!data) return;

    // Need either preimage auth or account auth with wallet
    if (!hasPreimageAuth && !hasAccountAuth) return;

    setIsUploading(true);
    setUploadError(null);
    setUploadResult(null);
    setTxStatus(null);

    try {
      // Calculate content hash for display
      const contentHash = await getContentHash(data, hashAlgorithm);
      const contentHashHex = bytesToHex(contentHash);

      if (hasPreimageAuth) {
        // Unsigned submission via raw PAPI (no wallet/signer needed)
        const isCustomCid = hashAlgorithm !== HashAlgorithm.Blake2b256 || cidCodec !== CidCodec.Raw;

        const toHashingEnum = (alg: HashAlgorithm) => {
          switch (alg) {
            case HashAlgorithm.Blake2b256: return { type: "Blake2b256" as const, value: undefined };
            case HashAlgorithm.Sha2_256: return { type: "Sha2_256" as const, value: undefined };
            case HashAlgorithm.Keccak256: return { type: "Keccak256" as const, value: undefined };
            default: return { type: "Blake2b256" as const, value: undefined };
          }
        };

        const tx = isCustomCid
          ? api.tx.TransactionStorage.store_with_cid_config({
              cid: {
                codec: BigInt(cidCodec),
                hashing: toHashingEnum(hashAlgorithm),
              },
              data: Binary.fromBytes(data),
            })
          : api.tx.TransactionStorage.store({
              data: Binary.fromBytes(data),
            });

        setTxStatus("Submitting unsigned transaction...");

        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const bareTx = await (tx as any).getBareTx();

        const result = await new Promise<{ blockHash?: string; blockNumber?: number; index?: number }>((resolve, reject) => {
          let resolved = false;

          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          const handleEvent = (ev: any) => {
            if (ev.type === "txBestBlocksState" && ev.found && !resolved) {
              resolved = true;
              subscription.unsubscribe();

              let index: number | undefined;
              if (ev.events) {
                // eslint-disable-next-line @typescript-eslint/no-explicit-any
                const storedEvent = ev.events.find((e: any) =>
                  e.type === "TransactionStorage" && e.value?.type === "Stored"
                );
                if (storedEvent?.value?.value?.index !== undefined) {
                  index = storedEvent.value.value.index;
                }
              }

              resolve({
                blockHash: ev.block.hash,
                blockNumber: ev.block.number,
                index,
              });
            }
          };

          const subscription = client.submitAndWatch(bareTx).subscribe({
            next: handleEvent,
            error: (err) => {
              if (!resolved) {
                resolved = true;
                reject(err);
              }
            },
          });

          setTimeout(() => {
            if (!resolved) {
              resolved = true;
              subscription.unsubscribe();
              reject(new Error("Transaction timed out"));
            }
          }, 120000);
        });

        // Calculate CID for display
        const { calculateCid } = await import("@bulletin/sdk");
        const cid = await calculateCid(data, cidCodec, hashAlgorithm);
        const cidStr = cid.toString();

        const uploadResultData: UploadResult = {
          cid: cidStr,
          contentHash: contentHashHex,
          blockHash: result.blockHash,
          blockNumber: result.blockNumber,
          index: result.index,
          size: data.length,
          unsigned: true,
        };

        setUploadResult(uploadResultData);
      } else {
        // Signed submission via SDK
        const bulletinClient = createBulletinClient!(selectedAccount!.polkadotSigner);

        const result = await bulletinClient
          .store(data)
          .withCodec(cidCodec)
          .withHashAlgorithm(hashAlgorithm)
          .withCallback(handleProgress)
          .withWaitFor(WaitFor.Finalized)
          .send();

        const cidStr = result.cid?.toString() ?? "";
        const uploadResultData: UploadResult = {
          cid: cidStr,
          contentHash: contentHashHex,
          blockNumber: result.blockNumber,
          index: result.extrinsicIndex,
          size: result.size,
        };

        setUploadResult(uploadResultData);

        // Save to history for easy renewal later (only for signed transactions)
        if (result.blockNumber !== undefined && result.extrinsicIndex !== undefined) {
          addStorageEntry({
            blockNumber: result.blockNumber,
            index: result.extrinsicIndex,
            cid: cidStr,
            contentHash: contentHashHex,
            size: result.size,
            account: selectedAccount!.address,
            networkId: network.id,
            label: fileName || undefined,
          });
        }
      }
    } catch (err) {
      console.error("Upload failed:", err);
      setUploadError(err instanceof Error ? err.message : "Upload failed");
    } finally {
      setIsUploading(false);
      setTxStatus(null);
    }
  };

  const copyToClipboard = async (text: string) => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Upload Data</h1>
        <p className="text-muted-foreground">
          Store data on the Bulletin Chain and receive an IPFS-compatible CID
        </p>
      </div>

      {/* Error Display - at top for visibility */}
      {uploadError && (
        <Card className="border-destructive relative">
          <Button
            variant="ghost"
            size="icon"
            className="absolute top-2 right-2 h-6 w-6 text-muted-foreground hover:text-foreground"
            onClick={() => setUploadError(null)}
          >
            <X className="h-4 w-4" />
          </Button>
          <CardContent className="pt-6 pr-10">
            <div className="flex items-start gap-3 text-destructive">
              <AlertCircle className="h-5 w-5 mt-0.5" />
              <div>
                <p className="font-medium">Upload Failed</p>
                <p className="text-sm mt-1">{uploadError}</p>
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Result Display - at top for visibility */}
      {uploadResult && (
        <Card className="border-success relative">
          <Button
            variant="ghost"
            size="icon"
            className="absolute top-2 right-2 h-6 w-6 text-muted-foreground hover:text-foreground"
            onClick={() => setUploadResult(null)}
          >
            <X className="h-4 w-4" />
          </Button>
          <CardHeader className="pr-10">
            <CardTitle className="text-success">Upload Successful</CardTitle>
            <CardDescription>
              Your data has been stored on the Bulletin Chain
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            {/* CID */}
            <div className="space-y-2">
              <label className="text-sm font-medium">CID (Content Identifier)</label>
              <div className="flex items-center gap-2">
                <Input
                  value={uploadResult.cid}
                  readOnly
                  className="font-mono text-sm"
                  data-testid="cid-display"
                />
                <Button
                  variant="outline"
                  size="icon"
                  onClick={() => copyToClipboard(uploadResult.cid)}
                >
                  {copied ? (
                    <Check className="h-4 w-4" />
                  ) : (
                    <Copy className="h-4 w-4" />
                  )}
                </Button>
              </div>
            </div>

            {/* Block & Index - Important for renewal */}
            <div className="p-3 rounded-md bg-primary/10 border border-primary/20">
              <div className="flex items-start gap-2 mb-2">
                <Info className="h-4 w-4 mt-0.5 text-primary" />
                <span className="text-sm font-medium">Save for Renewal</span>
              </div>
              <div className="grid grid-cols-2 gap-4 text-sm">
                <div>
                  <span className="text-muted-foreground">Block Number</span>
                  <p className="font-mono font-medium">#{uploadResult.blockNumber}</p>
                </div>
                <div>
                  <span className="text-muted-foreground">Transaction Index</span>
                  <p className="font-mono font-medium">{uploadResult.index ?? "N/A"}</p>
                </div>
              </div>
              <p className="text-xs text-muted-foreground mt-2">
                You'll need these values to renew your data before it expires.
                {uploadResult.blockNumber !== undefined && uploadResult.index !== undefined && (
                  <span className="text-success"> (Auto-saved to browser history)</span>
                )}
              </p>
            </div>

            {/* Additional Details */}
            <div className="grid sm:grid-cols-2 gap-4 text-sm">
              <div>
                <span className="text-muted-foreground">Size</span>
                <p>{formatBytes(uploadResult.size)}</p>
              </div>
              <div>
                <span className="text-muted-foreground">Content Hash</span>
                <p className="font-mono text-xs truncate" title={uploadResult.contentHash}>
                  {uploadResult.contentHash}
                </p>
              </div>
            </div>

            {/* Actions */}
            <div className="flex flex-wrap gap-2 pt-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => window.open(`https://ipfs.io/ipfs/${uploadResult.cid}`, "_blank")}
              >
                <ExternalLink className="h-4 w-4 mr-2" />
                View on IPFS
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => navigate(`/download?cid=${uploadResult.cid}`)}
              >
                <ExternalLink className="h-4 w-4 mr-2" />
                Download Page
              </Button>
              {uploadResult.blockNumber !== undefined && uploadResult.index !== undefined && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => navigate(`/renew?block=${uploadResult.blockNumber}&index=${uploadResult.index}`)}
                >
                  <RefreshCw className="h-4 w-4 mr-2" />
                  Renew Later
                </Button>
              )}
            </div>
          </CardContent>
        </Card>
      )}

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2 space-y-6">
          {/* Input Card */}
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <UploadIcon className="h-5 w-5" />
                Data Input
              </CardTitle>
              <CardDescription>
                Enter text or upload a file to store on-chain
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <Tabs value={inputMode} onValueChange={(v) => setInputMode(v as "text" | "file")}>
                <TabsList>
                  <TabsTrigger value="text">Text</TabsTrigger>
                  <TabsTrigger value="file">File</TabsTrigger>
                </TabsList>
                <TabsContent value="text" className="space-y-4">
                  <Textarea
                    placeholder="Enter data to store..."
                    value={textData}
                    onChange={(e) => {
                      setTextData(e.target.value);
                      setUploadResult(null);
                      setUploadError(null);
                    }}
                    className="min-h-[200px] font-mono"
                    disabled={isUploading}
                  />
                </TabsContent>
                <TabsContent value="file">
                  <FileUpload
                    onFileSelect={handleFileSelect}
                    maxSize={1024 * 1024} // 1MB
                    disabled={isUploading}
                  />
                  {fileName && (
                    <p className="text-sm text-muted-foreground mt-2">
                      Selected: {fileName}
                    </p>
                  )}
                </TabsContent>
              </Tabs>

              {dataSize > 0 && (
                <div className="flex items-center gap-2 text-sm text-muted-foreground">
                  <span>Data size:</span>
                  <Badge variant="secondary">{formatBytes(dataSize)}</Badge>
                </div>
              )}
            </CardContent>
          </Card>

          {/* Configuration Card */}
          <Card>
            <CardHeader>
              <CardTitle>CID Configuration</CardTitle>
              <CardDescription>
                Customize the CID format (optional)
              </CardDescription>
            </CardHeader>
            <CardContent>
              <div className="grid sm:grid-cols-2 gap-4">
                <div className="space-y-2">
                  <label className="text-sm font-medium">Hash Algorithm</label>
                  <Select value={String(hashAlgorithm)} onValueChange={(v) => setHashAlgorithm(Number(v) as HashAlgorithm)}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {HASH_ALGORITHMS.map((alg) => (
                        <SelectItem key={alg.value} value={String(alg.value)}>
                          {alg.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <label className="text-sm font-medium">CID Codec</label>
                  <Select value={String(cidCodec)} onValueChange={(v) => setCidCodec(Number(v) as CidCodec)}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {CID_CODECS.map((codec) => (
                        <SelectItem key={codec.value} value={String(codec.value)}>
                          {codec.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>
            </CardContent>
          </Card>

          {/* Upload Button */}
          <Button
            onClick={handleUpload}
            disabled={!canUpload || isUploading}
            className="w-full"
            size="lg"
            data-testid="upload-button"
          >
            {isUploading ? (
              <>
                <Spinner size="sm" className="mr-2" />
                {txStatus || "Uploading..."}
              </>
            ) : (
              <>
                <UploadIcon className="h-5 w-5 mr-2" />
                {willUseUnsigned ? "Upload (Unsigned)" : "Upload to Bulletin Chain"}
              </>
            )}
          </Button>
        </div>

        {/* Sidebar */}
        <div className="space-y-6">
          <AuthorizationCard />

          {(preimageAuth || preimageAuthLoading) && (
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center gap-2">
                  <Shield className="h-4 w-4" />
                  Preimage Authorization
                </CardTitle>
              </CardHeader>
              <CardContent>
                {preimageAuthLoading ? (
                  <div className="flex items-center gap-2 text-sm text-muted-foreground">
                    <Spinner size="sm" />
                    Checking...
                  </div>
                ) : preimageAuth ? (
                  <div className="space-y-1 text-sm">
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Transactions</span>
                      <span>{preimageAuth.transactions.toString()}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Bytes</span>
                      <span>{formatBytes(Number(preimageAuth.bytes))}</span>
                    </div>
                    <p className="text-xs text-muted-foreground mt-2">
                      No wallet required for upload
                    </p>
                  </div>
                ) : null}
              </CardContent>
            </Card>
          )}

          {!selectedAccount && !hasPreimageAuth && (
            <Card>
              <CardContent className="pt-6">
                <div className="text-center text-muted-foreground">
                  <p className="mb-4">Connect a wallet to upload data</p>
                  <Button variant="outline" asChild>
                    <a href="/accounts">Connect Wallet</a>
                  </Button>
                </div>
              </CardContent>
            </Card>
          )}

          {selectedAccount && !authorization && !hasPreimageAuth && (
            <Card>
              <CardContent className="pt-6">
                <div className="text-center text-muted-foreground">
                  <AlertCircle className="h-8 w-8 mx-auto mb-2" />
                  <p>No authorization found</p>
                  <p className="text-sm mt-2">
                    Contact an admin to get storage authorization
                  </p>
                </div>
              </CardContent>
            </Card>
          )}
        </div>
      </div>
    </div>
  );
}
