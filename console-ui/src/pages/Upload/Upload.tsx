import { useState, useCallback, useEffect, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { Upload as UploadIcon, Copy, Check, ExternalLink, AlertCircle, RefreshCw, Info, X, FileText, Shield, Archive } from "lucide-react";
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
import { FileUpload, type SelectedFile } from "@/components/FileUpload";
import { createCarArchive, type CarResult } from "@/lib/car";
import { CHUNK_SIZE, CHUNKED_THRESHOLD, chunkData, buildChunkMetadata, buildUnixFSDag } from "@/lib/chunked-upload";
import { AuthorizationCard } from "@/components/AuthorizationCard";
import { useApi, useClient, useChainState } from "@/state/chain.state";
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
  contentHash: string;
  blockHash?: string;
  blockNumber?: number;
  index?: number;
  size: number;
  unsigned?: boolean;
  carRootCid?: string;
  carFiles?: { name: string; cid: string; size: number }[];
  isChunked?: boolean;
  metadataCid?: string;
  dagRootCid?: string;
  chunks?: { cid: string; size: number; blockNumber?: number }[];
}

interface UploadProgress {
  phase: string;
  current: number;
  total: number;
}

async function submitTransaction(
  api: any,
  polkadotSigner: any,
  data: Uint8Array,
  hashCode: number,
  codec: number,
): Promise<{ blockHash?: string; blockNumber?: number; index?: number }> {
  const isCustomCid = hashCode !== 0xb220 || codec !== 0x55;

  const tx = isCustomCid
    ? api.tx.TransactionStorage.store_with_cid_config({
        cid: { codec: BigInt(codec), hashing: toHashingEnum(hashCode) },
        data: Binary.fromBytes(data),
      })
    : api.tx.TransactionStorage.store({ data: Binary.fromBytes(data) });

  return new Promise((resolve, reject) => {
    let resolved = false;
    let subscription: { unsubscribe: () => void };

    const handleEvent = (ev: any) => {
      console.log("TX event:", ev.type);
      if (ev.type === "txBestBlocksState" && ev.found && !resolved) {
        resolved = true;
        subscription.unsubscribe();
        let index: number | undefined;
        if (ev.events) {
          const storedEvent = ev.events.find(
            (e: any) => e.type === "TransactionStorage" && e.value?.type === "Stored"
          );
          if (storedEvent?.value?.value?.index !== undefined) {
            index = storedEvent.value.value.index;
          }
        }
        resolve({ blockHash: ev.block.hash, blockNumber: ev.block.number, index });
      }
    };

    const handleError = (err: any) => {
      if (!resolved) {
        resolved = true;
        reject(err);
      }
    };

    subscription = tx.signSubmitAndWatch(polkadotSigner).subscribe({
      next: handleEvent,
      error: handleError,
    });

    setTimeout(() => {
      if (!resolved) {
        resolved = true;
        subscription?.unsubscribe();
        reject(new Error("Transaction timed out"));
      }
    }, 120000);
  });
}

export function Upload() {
  const api = useApi();
  const client = useClient();
  const { network } = useChainState();
  const navigate = useNavigate();
  const selectedAccount = useSelectedAccount();
  const authorization = useAuthorization();
  const preimageAuth = usePreimageAuth();
  const preimageAuthLoading = usePreimageAuthLoading();

  const [inputMode, setInputMode] = useState<"text" | "file">("text");
  const [textData, setTextData] = useState("");
  const [selectedFiles, setSelectedFiles] = useState<SelectedFile[]>([]);
  const [carData, setCarData] = useState<CarResult | null>(null);
  const [isBuildingCar, setIsBuildingCar] = useState(false);
  const [carError, setCarError] = useState<string | null>(null);

  const [hashAlgorithm, setHashAlgorithm] = useState<HashAlgorithm>("blake2b256");
  const [cidCodec, setCidCodec] = useState("raw");

  const [isUploading, setIsUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [uploadResult, setUploadResult] = useState<UploadResult | null>(null);
  const [copied, setCopied] = useState(false);
  const [uploadProgress, setUploadProgress] = useState<UploadProgress | null>(null);

  const debounceTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  const getData = useCallback((): Uint8Array | null => {
    if (inputMode === "text") {
      if (!textData.trim()) return null;
      return new TextEncoder().encode(textData);
    }
    if (selectedFiles.length === 0) return null;
    if (selectedFiles.length === 1) return selectedFiles[0]!.data;
    return carData?.carBytes ?? null;
  }, [inputMode, textData, selectedFiles, carData]);

  const dataSize = getData()?.length ?? 0;
  const rawFilesSize = selectedFiles.reduce((sum, f) => sum + f.data.length, 0);
  const filesLabel = selectedFiles.length === 1
    ? selectedFiles[0]!.file.name
    : selectedFiles.length > 1
      ? `${selectedFiles.length} files (CAR archive)`
      : null;

  // Auto-select raw codec for CAR archives (multiple files)
  const isCarUpload = selectedFiles.length > 1;
  const isChunkedUpload = isCarUpload && dataSize > CHUNKED_THRESHOLD;
  const estimatedChunks = isChunkedUpload ? Math.ceil(dataSize / CHUNK_SIZE) : 0;
  const estimatedTransactions = isChunkedUpload ? estimatedChunks + 2 : 1; // +2 for metadata + DAG

  useEffect(() => {
    if (isCarUpload) {
      setCidCodec("raw");
    }
  }, [isCarUpload]);

  // Build CAR archive when multiple files are selected
  useEffect(() => {
    if (selectedFiles.length <= 1) {
      setCarData(null);
      setCarError(null);
      return;
    }

    let cancelled = false;
    setIsBuildingCar(true);
    setCarError(null);
    setCarData(null);

    createCarArchive(selectedFiles.map(f => ({ name: f.relativePath, data: f.data })))
      .then(result => {
        if (!cancelled) {
          setCarData(result);
          setIsBuildingCar(false);
        }
      })
      .catch(err => {
        if (!cancelled) {
          setCarError(err instanceof Error ? err.message : "Failed to build CAR archive");
          setIsBuildingCar(false);
        }
      });

    return () => { cancelled = true; };
  }, [selectedFiles]);

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
  }, [api, inputMode, textData, selectedFiles, carData, hashAlgorithm, getData]);

  const hasAccountAuth =
    selectedAccount &&
    authorization &&
    authorization.bytes >= BigInt(dataSize) &&
    authorization.transactions >= BigInt(estimatedTransactions);

  // Chunked uploads don't support preimage auth (each chunk has a different content hash)
  const hasPreimageAuth =
    !isChunkedUpload &&
    preimageAuth &&
    preimageAuth.bytes >= BigInt(dataSize) &&
    preimageAuth.transactions > 0n;

  const canUpload =
    api &&
    client &&
    dataSize > 0 &&
    !isBuildingCar &&
    (hasAccountAuth || hasPreimageAuth);

  // Preimage auth is preferred (same as pallet behavior)
  const willUseUnsigned = !!hasPreimageAuth;

  const handleFilesSelect = useCallback((files: SelectedFile[]) => {
    setSelectedFiles(files);
    setUploadResult(null);
    setUploadError(null);
    setCarError(null);
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
    setUploadProgress(null);

    try {
      const hashConfig = HASH_ALGORITHMS.find(h => h.value === hashAlgorithm);
      const codecConfig = CID_CODECS.find(c => c.value === cidCodec);

      if (!hashConfig || !codecConfig) {
        throw new Error("Invalid configuration");
      }

      const useUnsigned = hasPreimageAuth;

      if (isChunkedUpload) {
        // ── Chunked upload flow ──────────────────────────────────────
        const signer = selectedAccount!.polkadotSigner;
        const totalSteps = Math.ceil(data.length / CHUNK_SIZE) + 2; // chunks + metadata + DAG

        // 1. Chunk the data
        setUploadProgress({ phase: "Splitting into chunks...", current: 0, total: totalSteps });
        const chunks = await chunkData(data, codecConfig.codec, hashConfig.mhCode);

        // 2. Store each chunk as a separate transaction
        const storedChunks: { cid: string; size: number; blockNumber?: number }[] = [];
        for (let i = 0; i < chunks.length; i++) {
          setUploadProgress({
            phase: `Uploading chunk ${i + 1}/${chunks.length}`,
            current: i + 1,
            total: totalSteps,
          });
          const result = await submitTransaction(
            api, signer, chunks[i]!.data, hashConfig.mhCode, codecConfig.codec,
          );
          storedChunks.push({
            cid: chunks[i]!.cid.toString(),
            size: chunks[i]!.size,
            blockNumber: result.blockNumber,
          });
        }

        // 3. Build and store metadata with all chunk CIDs
        setUploadProgress({
          phase: "Uploading metadata",
          current: chunks.length + 1,
          total: totalSteps,
        });
        const metadataBytes = buildChunkMetadata(storedChunks, data.length);
        const metadataCid = await cidFromBytes(metadataBytes, codecConfig.codec, hashConfig.mhCode);
        const contentHash = await getContentHash(metadataBytes, hashConfig.mhCode);
        const contentHashHex = "0x" + Array.from(contentHash).map(b => b.toString(16).padStart(2, "0")).join("");

        const metaResult = await submitTransaction(
          api, signer, metadataBytes, hashConfig.mhCode, codecConfig.codec,
        );

        // 4. Build UnixFS DAG-PB node linking all chunks and store it.
        //    This makes the content accessible via a single IPFS root CID.
        //    Stored with dag-pb codec (0x70) so IPFS can traverse the links.
        setUploadProgress({
          phase: "Building and storing DAG",
          current: chunks.length + 2,
          total: totalSteps,
        });
        const { rootCid: dagRootCid, dagBytes } = await buildUnixFSDag(storedChunks, hashConfig.mhCode);
        await submitTransaction(
          api, signer, dagBytes, hashConfig.mhCode, 0x70, // dag-pb codec
        );

        setUploadProgress(null);

        const uploadResultData: UploadResult = {
          cid: metadataCid.toString(),
          contentHash: contentHashHex,
          blockHash: metaResult.blockHash,
          blockNumber: metaResult.blockNumber,
          index: metaResult.index,
          size: data.length,
          unsigned: false,
          carRootCid: carData?.rootCid.toString(),
          carFiles: carData?.files,
          isChunked: true,
          metadataCid: metadataCid.toString(),
          dagRootCid: dagRootCid.toString(),
          chunks: storedChunks,
        };

        setUploadResult(uploadResultData);

        if (selectedAccount && metaResult.blockNumber !== undefined && metaResult.index !== undefined) {
          addStorageEntry({
            blockNumber: metaResult.blockNumber,
            index: metaResult.index,
            cid: dagRootCid.toString(),
            contentHash: contentHashHex,
            size: data.length,
            account: selectedAccount.address,
            networkId: network.id,
            label: `${selectedFiles.length} files (chunked CAR)`,
          });
        }
      } else {
        // ── Single transaction flow ──────────────────────────────────
        const expectedCid = await cidFromBytes(data, codecConfig.codec, hashConfig.mhCode);
        const contentHash = await getContentHash(data, hashConfig.mhCode);
        const contentHashHex = "0x" + Array.from(contentHash).map(b => b.toString(16).padStart(2, "0")).join("");

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

        const result = await new Promise<{ blockHash?: string; blockNumber?: number; index?: number }>((resolve, reject) => {
          let resolved = false;

          const handleEvent = (ev: any) => {
            console.log("TX event:", ev.type);
            if (ev.type === "txBestBlocksState" && ev.found && !resolved) {
              resolved = true;
              subscription.unsubscribe();

              let index: number | undefined;
              if (ev.events) {
                const storedEvent = ev.events.find(
                  (e: any) => e.type === "TransactionStorage" && e.value?.type === "Stored"
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

          const handleError = (err: any) => {
            if (!resolved) {
              resolved = true;
              reject(err);
            }
          };

          let subscription: { unsubscribe: () => void };

          if (useUnsigned) {
            tx.getBareTx().then((bareTx) => {
              subscription = client.submitAndWatch(bareTx).subscribe({
                next: handleEvent,
                error: handleError,
              });
            }).catch(handleError);
          } else {
            subscription = tx.signSubmitAndWatch(selectedAccount!.polkadotSigner).subscribe({
              next: handleEvent,
              error: handleError,
            });
          }

          setTimeout(() => {
            if (!resolved) {
              resolved = true;
              subscription?.unsubscribe();
              reject(new Error("Transaction timed out"));
            }
          }, 120000);
        });

        const uploadResultData: UploadResult = {
          cid: expectedCid.toString(),
          contentHash: contentHashHex,
          blockHash: result.blockHash,
          blockNumber: result.blockNumber,
          index: result.index,
          size: data.length,
          unsigned: !!useUnsigned,
          carRootCid: carData?.rootCid.toString(),
          carFiles: carData?.files,
        };

        setUploadResult(uploadResultData);

        if (!useUnsigned && selectedAccount && result.blockNumber !== undefined && result.index !== undefined) {
          addStorageEntry({
            blockNumber: result.blockNumber,
            index: result.index,
            cid: expectedCid.toString(),
            contentHash: contentHashHex,
            size: data.length,
            account: selectedAccount.address,
            networkId: network.id,
            label: filesLabel || undefined,
          });
        }
      }
    } catch (err) {
      console.error("Upload failed:", err);
      setUploadError(err instanceof Error ? err.message : "Upload failed");
    } finally {
      setIsUploading(false);
      setUploadProgress(null);
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
              {uploadResult.isChunked
                ? `Your data has been stored in ${uploadResult.chunks?.length ?? 0} chunks on the Bulletin Chain`
                : "Your data has been stored on the Bulletin Chain"}
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            {/* CID */}
            <div className="space-y-2">
              <label className="text-sm font-medium">
                {uploadResult.isChunked ? "Metadata CID" : "CID (Content Identifier)"}
              </label>
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

            {/* DAG Root CID — the IPFS-accessible CID for chunked uploads */}
            {uploadResult.dagRootCid && (
              <div className="space-y-2">
                <label className="text-sm font-medium">IPFS Root CID (DAG)</label>
                <div className="flex items-center gap-2">
                  <Input
                    value={uploadResult.dagRootCid}
                    readOnly
                    className="font-mono text-sm"
                  />
                  <Button
                    variant="outline"
                    size="icon"
                    onClick={() => copyToClipboard(uploadResult.dagRootCid!)}
                  >
                    {copied ? (
                      <Check className="h-4 w-4" />
                    ) : (
                      <Copy className="h-4 w-4" />
                    )}
                  </Button>
                </div>
                <p className="text-xs text-muted-foreground">
                  Use this CID to access the reassembled content on IPFS
                </p>
              </div>
            )}

            {/* CAR Archive Info */}
            {uploadResult.carRootCid && (
              <div className="space-y-2">
                <label className="text-sm font-medium flex items-center gap-2">
                  <Archive className="h-4 w-4" />
                  CAR Archive
                </label>
                <div className="rounded-md border bg-muted/50 p-3 space-y-2">
                  <div>
                    <span className="text-xs text-muted-foreground">Root CID (UnixFS directory)</span>
                    <p className="font-mono text-xs break-all">{uploadResult.carRootCid}</p>
                  </div>
                  {uploadResult.carFiles && uploadResult.carFiles.length > 0 && (
                    <div>
                      <span className="text-xs text-muted-foreground">
                        {uploadResult.carFiles.length} file{uploadResult.carFiles.length !== 1 ? "s" : ""} in archive
                      </span>
                      <div className="mt-1 space-y-1">
                        {uploadResult.carFiles.map((f, i) => (
                          <div key={i} className="flex items-center justify-between text-xs">
                            <span className="font-medium truncate mr-2">{f.name}</span>
                            <span className="text-muted-foreground shrink-0">{formatBytes(f.size)}</span>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              </div>
            )}

            {/* Chunked Upload Details */}
            {uploadResult.isChunked && uploadResult.chunks && (
              <div className="space-y-2">
                <label className="text-sm font-medium flex items-center gap-2">
                  <Archive className="h-4 w-4" />
                  Chunked Upload
                </label>
                <div className="rounded-md border bg-muted/50 p-3 space-y-2">
                  <div className="flex items-center justify-between text-sm">
                    <span className="text-muted-foreground">
                      {uploadResult.chunks.length} chunks stored
                    </span>
                    <span className="text-muted-foreground">
                      {formatBytes(uploadResult.size)} total
                    </span>
                  </div>
                  <div className="mt-1 space-y-1 max-h-32 overflow-y-auto">
                    {uploadResult.chunks.map((c, i) => (
                      <div key={i} className="flex items-center justify-between text-xs">
                        <span className="font-mono truncate mr-2">Chunk {i + 1}: {c.cid.slice(0, 24)}...</span>
                        <span className="text-muted-foreground shrink-0">{formatBytes(c.size)}</span>
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            )}

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
                onClick={() => window.open(`https://${uploadResult.dagRootCid || uploadResult.cid}.app.dot.li`, "_blank")}
              >
                <ExternalLink className="h-4 w-4 mr-2" />
                View on Dot.li
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => navigate(`/download?cid=${uploadResult.dagRootCid || uploadResult.cid}`)}
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
                <TabsContent value="file" className="space-y-3">
                  <FileUpload
                    multiple
                    onFilesSelect={handleFilesSelect}
                    maxSize={5 * 1024 * 1024} // 5MB per file (chunked for totals > 2MB)
                    disabled={isUploading || isBuildingCar}
                  />
                  {selectedFiles.length > 1 && (
                    <div className="flex items-center gap-2 text-sm">
                      <Archive className="h-4 w-4 text-muted-foreground" />
                      {isBuildingCar ? (
                        <span className="text-muted-foreground flex items-center gap-2">
                          <Spinner size="sm" /> Building CAR archive...
                        </span>
                      ) : carData ? (
                        <span className="text-muted-foreground">
                          CAR archive ready — {formatBytes(carData.carBytes.length)}
                        </span>
                      ) : carError ? (
                        <span className="text-destructive">{carError}</span>
                      ) : null}
                    </div>
                  )}
                </TabsContent>
              </Tabs>

              {(dataSize > 0 || rawFilesSize > 0) && (
                <div className="flex items-center gap-2 text-sm text-muted-foreground">
                  <span>Data size:</span>
                  <Badge variant="secondary">
                    {dataSize > 0
                      ? formatBytes(dataSize)
                      : `~${formatBytes(rawFilesSize)} (building CAR...)`
                    }
                  </Badge>
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
              {isCarUpload && (
                <div className="flex items-start gap-2 p-3 rounded-md bg-muted/50 border text-sm mb-4">
                  <Info className="h-4 w-4 mt-0.5 text-primary shrink-0" />
                  <div>
                    <p className="font-medium">CAR archive detected</p>
                    <p className="text-muted-foreground text-xs mt-1">
                      Multiple files will be packed into a CAR (Content Addressable Archive).
                      The codec is set to <strong>Raw</strong> because the chain stores the CAR as a binary blob.
                      The UnixFS directory root CID (DAG-PB) is shown separately after upload.
                    </p>
                    {isChunkedUpload && (
                      <p className="text-primary text-xs mt-1 font-medium">
                        Data exceeds 2 MB — will be uploaded in {estimatedTransactions} transactions
                        (~{estimatedChunks} chunks + metadata + DAG).
                      </p>
                    )}
                  </div>
                </div>
              )}
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
                  <Select value={cidCodec} onValueChange={setCidCodec} disabled={isCarUpload}>
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
                {uploadProgress
                  ? uploadProgress.phase
                  : willUseUnsigned ? "Uploading (unsigned)..." : "Uploading..."}
              </>
            ) : (
              <>
                <UploadIcon className="h-5 w-5 mr-2" />
                {willUseUnsigned
                  ? "Upload to Bulletin Chain (unsigned)"
                  : isChunkedUpload
                    ? `Upload to Bulletin Chain (${estimatedTransactions} transactions)`
                    : "Upload to Bulletin Chain"}
              </>
            )}
          </Button>

          {/* Progress bar for chunked uploads */}
          {uploadProgress && uploadProgress.total > 0 && (
            <div className="w-full space-y-1">
              <div className="flex items-center justify-between text-sm text-muted-foreground">
                <span>{uploadProgress.phase}</span>
                <span>{uploadProgress.current}/{uploadProgress.total}</span>
              </div>
              <div className="h-2 bg-muted rounded-full overflow-hidden">
                <div
                  className="h-full bg-primary transition-all duration-300 rounded-full"
                  style={{ width: `${(uploadProgress.current / uploadProgress.total) * 100}%` }}
                />
              </div>
            </div>
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
                ) : isChunkedUpload && preimageAuth ? (
                  <div className="space-y-2 py-2">
                    <p className="text-sm text-amber-600">
                      Preimage authorization cannot be used for chunked uploads.
                    </p>
                    <p className="text-xs text-muted-foreground">
                      Each chunk has a different content hash. Use account authorization instead.
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
