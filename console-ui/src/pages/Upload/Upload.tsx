import { useState, useCallback } from "react";
import { Upload as UploadIcon, Copy, Check, ExternalLink, AlertCircle } from "lucide-react";
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
import { useAuthorization } from "@/state/storage.state";
import { formatBytes } from "@/utils/format";
import { cidFromBytes, toHashingEnum } from "@/lib/cid";
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
}

export function Upload() {
  const api = useApi();
  const client = useClient();
  const selectedAccount = useSelectedAccount();
  const authorization = useAuthorization();

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

  const getData = useCallback((): Uint8Array | null => {
    if (inputMode === "text") {
      if (!textData.trim()) return null;
      return new TextEncoder().encode(textData);
    }
    return fileData;
  }, [inputMode, textData, fileData]);

  const dataSize = getData()?.length ?? 0;

  const canUpload =
    api &&
    selectedAccount &&
    authorization &&
    dataSize > 0 &&
    authorization.bytes >= BigInt(dataSize) &&
    authorization.transactions > 0n;

  const handleFileSelect = useCallback((file: File | null, data: Uint8Array | null) => {
    setFileData(data);
    setFileName(file?.name ?? null);
    setUploadResult(null);
    setUploadError(null);
  }, []);

  const handleUpload = async () => {
    if (!api || !selectedAccount || !client) return;

    const data = getData();
    if (!data) return;

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

      // Build transaction options with custom extension if non-default config
      const txOpts: Record<string, unknown> = {};
      if (hashAlgorithm !== "blake2b256" || cidCodec !== "raw") {
        txOpts.customSignedExtensions = {
          ProvideCidConfig: {
            value: {
              codec: BigInt(codecConfig.codec),
              hashing: toHashingEnum(hashConfig.mhCode),
            },
          },
        };
      }

      // Create and submit transaction
      const tx = api.tx.TransactionStorage.store({
        data: Binary.fromBytes(data),
      });

      // Sign and submit
      const result = await new Promise<{ blockHash?: string; blockNumber?: number }>((resolve, reject) => {
        let resolved = false;

        const subscription = tx.signSubmitAndWatch(selectedAccount.polkadotSigner, txOpts).subscribe({
          next: (ev) => {
            console.log("TX event:", ev.type);
            if (ev.type === "txBestBlocksState" && ev.found && !resolved) {
              resolved = true;
              subscription.unsubscribe();
              resolve({
                blockHash: ev.block.hash,
                blockNumber: ev.block.number,
              });
            }
          },
          error: (err) => {
            if (!resolved) {
              resolved = true;
              reject(err);
            }
          },
        });

        // Timeout after 2 minutes
        setTimeout(() => {
          if (!resolved) {
            resolved = true;
            subscription.unsubscribe();
            reject(new Error("Transaction timed out"));
          }
        }, 120000);
      });

      setUploadResult({
        cid: expectedCid.toString(),
        blockHash: result.blockHash,
        blockNumber: result.blockNumber,
        size: data.length,
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
                Uploading...
              </>
            ) : (
              <>
                <UploadIcon className="h-5 w-5 mr-2" />
                Upload to Bulletin Chain
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
                <CardTitle className="text-success">Upload Successful</CardTitle>
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
                    onClick={() => window.open(`/download?cid=${uploadResult.cid}`, "_blank")}
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
          <AuthorizationCard />

          {!selectedAccount && (
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

          {selectedAccount && !authorization && (
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
