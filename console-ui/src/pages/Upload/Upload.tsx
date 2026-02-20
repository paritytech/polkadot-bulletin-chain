import { useState, useCallback, useEffect, useRef } from "react";
import { Upload as UploadIcon, Copy, Check, ExternalLink, AlertCircle, FileText, Shield } from "lucide-react";
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
import { useApi, useClient } from "@/state/chain.state";
import { useSelectedAccount } from "@/state/wallet.state";
import {
  useAuthorization,
  usePreimageAuth,
  usePreimageAuthLoading,
  checkPreimageAuthorization,
  clearPreimageAuth,
} from "@/state/storage.state";
import { formatBytes } from "@/utils/format";
import { cidFromBytes, toHashingEnum, getContentHash } from "@/lib/cid";
import { Binary } from "polkadot-api";

type HashAlgorithm = "blake2b256" | "sha256" | "keccak256";

const HASH_ALGORITHMS: { value: HashAlgorithm; label: string; mhCode: number }[] = [
  { value: "blake2b256", label: "Blake2b-256 (default)", mhCode: 0xb220 },
  { value: "sha256", label: "SHA2-256", mhCode: 0x12 },
  { value: "keccak256", label: "Keccak-256", mhCode: 0x1b },
];

const CID_CODECS = [
  { value: "raw", label: "Raw (0x55)", codec: 0x55 },
  { value: "dag-pb", label: "DAG-PB (0x70)", codec: 0x70 },
];

interface UploadResult {
  cid: string;
  blockHash?: string;
  blockNumber?: number;
  size: number;
  unsigned?: boolean;
}

export function Upload() {
  const api = useApi();
  const client = useClient();
  const selectedAccount = useSelectedAccount();
  const authorization = useAuthorization();
  const preimageAuth = usePreimageAuth();
  const preimageAuthLoading = usePreimageAuthLoading();

  const [inputMode, setInputMode] = useState<"text" | "file">("text");
  const [textData, setTextData] = useState("");
  const [fileData, setFileData] = useState<Uint8Array | null>(null);
  const [fileName, setFileName] = useState<string | null>(null);

  const [hashAlgorithm, setHashAlgorithm] = useState<HashAlgorithm>("blake2b256");
  const [cidCodec, setCidCodec] = useState("raw");

  const [isUploading, setIsUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [uploadResult, setUploadResult] = useState<UploadResult | null>(null);
  const [copied, setCopied] = useState(false);

  const debounceTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  const getData = useCallback((): Uint8Array | null => {
    if (inputMode === "text") {
      if (!textData.trim()) return null;
      return new TextEncoder().encode(textData);
    }
    return fileData;
  }, [inputMode, textData, fileData]);

  const dataSize = getData()?.length ?? 0;

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
      const hashConfig = HASH_ALGORITHMS.find(h => h.value === hashAlgorithm);
      if (!hashConfig) return;

      try {
        const contentHash = await getContentHash(data, hashConfig.mhCode);
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
    selectedAccount &&
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

  // Preimage auth is preferred (same as pallet behavior)
  const willUseUnsigned = !!hasPreimageAuth;

  const handleFileSelect = useCallback((file: File | null, data: Uint8Array | null) => {
    setFileData(data);
    setFileName(file?.name ?? null);
    setUploadResult(null);
    setUploadError(null);
  }, []);

  const handleUpload = async () => {
    if (!api || !client) return;

    const data = getData();
    if (!data) return;

    // Need either preimage auth or account auth with wallet
    if (!hasPreimageAuth && !hasAccountAuth) return;
    if (!hasPreimageAuth && !selectedAccount) return;

    setIsUploading(true);
    setUploadError(null);
    setUploadResult(null);

    try {
      const hashConfig = HASH_ALGORITHMS.find(h => h.value === hashAlgorithm);
      const codecConfig = CID_CODECS.find(c => c.value === cidCodec);

      if (!hashConfig || !codecConfig) {
        throw new Error("Invalid configuration");
      }

      // Calculate expected CID
      const expectedCid = await cidFromBytes(data, codecConfig.codec, hashConfig.mhCode);

      // Use store_with_cid_config for non-default CID settings, plain store otherwise
      const isCustomCid = hashAlgorithm !== "blake2b256" || cidCodec !== "raw";

      const tx = isCustomCid
        ? api.tx.TransactionStorage.store_with_cid_config({
            cid: {
              codec: BigInt(codecConfig.codec),
              hashing: toHashingEnum(hashConfig.mhCode),
            },
            data: Binary.fromBytes(data),
          })
        : api.tx.TransactionStorage.store({
            data: Binary.fromBytes(data),
          });

      const useUnsigned = hasPreimageAuth;

      const result = await new Promise<{ blockHash?: string; blockNumber?: number }>((resolve, reject) => {
        let resolved = false;

        const handleEvent = (ev: any) => {
          console.log("TX event:", ev.type);
          if (ev.type === "txBestBlocksState" && ev.found && !resolved) {
            resolved = true;
            subscription.unsubscribe();
            resolve({
              blockHash: ev.block.hash,
              blockNumber: ev.block.number,
            });
          }
        };

        const handleError = (err: any) => {
          if (!resolved) {
            resolved = true;
            reject(err);
          }
        };

        let subscription: { unsubscribe: () => void };

        if (useUnsigned) {
          // Unsigned submission via bareTx
          tx.getBareTx().then((bareTx) => {
            subscription = client.submitAndWatch(bareTx).subscribe({
              next: handleEvent,
              error: handleError,
            });
          }).catch(handleError);
        } else {
          // Signed submission
          subscription = tx.signSubmitAndWatch(selectedAccount!.polkadotSigner).subscribe({
            next: handleEvent,
            error: handleError,
          });
        }

        // Timeout after 2 minutes
        setTimeout(() => {
          if (!resolved) {
            resolved = true;
            subscription?.unsubscribe();
            reject(new Error("Transaction timed out"));
          }
        }, 120000);
      });

      setUploadResult({
        cid: expectedCid.toString(),
        blockHash: result.blockHash,
        blockNumber: result.blockNumber,
        size: data.length,
        unsigned: !!useUnsigned,
      });
    } catch (err) {
      console.error("Upload failed:", err);
      setUploadError(err instanceof Error ? err.message : "Upload failed");
    } finally {
      setIsUploading(false);
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
                  <Select value={hashAlgorithm} onValueChange={(v) => setHashAlgorithm(v as HashAlgorithm)}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {HASH_ALGORITHMS.map((alg) => (
                        <SelectItem key={alg.value} value={alg.value}>
                          {alg.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <label className="text-sm font-medium">CID Codec</label>
                  <Select value={cidCodec} onValueChange={setCidCodec}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {CID_CODECS.map((codec) => (
                        <SelectItem key={codec.value} value={codec.value}>
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
          >
            {isUploading ? (
              <>
                <Spinner size="sm" className="mr-2" />
                {willUseUnsigned ? "Uploading (unsigned)..." : "Uploading..."}
              </>
            ) : (
              <>
                <UploadIcon className="h-5 w-5 mr-2" />
                {willUseUnsigned
                  ? "Upload to Bulletin Chain (unsigned)"
                  : "Upload to Bulletin Chain"}
              </>
            )}
          </Button>

          {/* Error Display */}
          {uploadError && (
            <Card className="border-destructive">
              <CardContent className="pt-6">
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

          {/* Result Display */}
          {uploadResult && (
            <Card className="border-success">
              <CardHeader>
                <CardTitle className="text-success flex items-center gap-2">
                  Upload Successful
                  {uploadResult.unsigned && (
                    <Badge variant="secondary">Unsigned</Badge>
                  )}
                </CardTitle>
                <CardDescription>
                  Your data has been stored on the Bulletin Chain
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="space-y-2">
                  <label className="text-sm font-medium">CID</label>
                  <div className="flex items-center gap-2">
                    <Input
                      value={uploadResult.cid}
                      readOnly
                      className="font-mono text-sm"
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

                <div className="grid grid-cols-2 gap-4 text-sm">
                  {uploadResult.blockNumber && (
                    <div>
                      <span className="text-muted-foreground">Block Number</span>
                      <p className="font-mono">#{uploadResult.blockNumber}</p>
                    </div>
                  )}
                  <div>
                    <span className="text-muted-foreground">Size</span>
                    <p>{formatBytes(uploadResult.size)}</p>
                  </div>
                </div>

                <div className="flex gap-2 pt-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => window.open(`${import.meta.env.BASE_URL}download?cid=${uploadResult.cid}`, "_blank")}
                  >
                    <ExternalLink className="h-4 w-4 mr-2" />
                    View in Download
                  </Button>
                </div>
              </CardContent>
            </Card>
          )}
        </div>

        {/* Sidebar */}
        <div className="space-y-6">
          {/* Preimage Authorization Card */}
          {dataSize > 0 && (
            <Card className={hasPreimageAuth ? "border-green-500/50" : undefined}>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <FileText className="h-5 w-5" />
                  Preimage Authorization
                </CardTitle>
                <CardDescription>
                  No wallet required for pre-authorized data
                </CardDescription>
              </CardHeader>
              <CardContent>
                {preimageAuthLoading ? (
                  <div className="flex items-center justify-center h-16">
                    <Spinner size="sm" />
                  </div>
                ) : hasPreimageAuth ? (
                  <div className="space-y-3">
                    <div className="flex items-center gap-2">
                      <Badge variant="default" className="bg-green-600">Authorized</Badge>
                      <span className="text-sm text-muted-foreground">
                        Up to {formatBytes(preimageAuth!.bytes)}
                      </span>
                    </div>
                    {preimageAuth!.expiresAt && (
                      <p className="text-xs text-muted-foreground">
                        Expires at block #{preimageAuth!.expiresAt}
                      </p>
                    )}
                    <p className="text-xs text-muted-foreground">
                      This data can be uploaded without a wallet connection
                    </p>
                  </div>
                ) : (
                  <div className="text-center text-muted-foreground py-2">
                    <p className="text-sm">No preimage authorization for this data</p>
                  </div>
                )}
              </CardContent>
            </Card>
          )}

          <AuthorizationCard />

          {!selectedAccount && !hasPreimageAuth && (
            <Card>
              <CardContent className="pt-6">
                <div className="text-center text-muted-foreground">
                  <p className="mb-4">Connect a wallet or use pre-authorized data to upload</p>
                  <Button variant="outline" asChild>
                    <a href={`${import.meta.env.BASE_URL}accounts`}>Connect Wallet</a>
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

          {/* Submission mode indicator */}
          {canUpload && (
            <Card>
              <CardContent className="pt-6">
                <div className="flex items-center gap-2 text-sm">
                  {willUseUnsigned ? (
                    <>
                      <FileText className="h-4 w-4 text-green-600" />
                      <span>Will submit as <strong>unsigned</strong> transaction (preimage authorized)</span>
                    </>
                  ) : (
                    <>
                      <Shield className="h-4 w-4 text-blue-600" />
                      <span>Will submit as <strong>signed</strong> transaction (account authorized)</span>
                    </>
                  )}
                </div>
              </CardContent>
            </Card>
          )}
        </div>
      </div>
    </div>
  );
}
