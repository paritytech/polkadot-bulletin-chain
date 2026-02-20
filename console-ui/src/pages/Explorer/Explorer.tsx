import { useState, useEffect, useCallback } from "react";
import { Search, ChevronLeft, ChevronRight, RefreshCw, Box, Hash } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import { useApi, useClient, useBlockNumber, useChainState } from "@/state/chain.state";
import { formatBlockNumber } from "@/utils/format";
import { Binary, type HexString } from "polkadot-api";

interface BlockInfo {
  number: number;
  hash: string;
  extrinsicsCount: number;
}

interface ExtrinsicInfo {
  index: number;
  pallet: string;
  call: string;
}

// Return the byte length of a SCALE compact-encoded integer at the given offset.
function compactLen(bytes: Uint8Array, offset: number): number {
  const mode = bytes[offset]! & 0x03;
  if (mode === 0) return 1;
  if (mode === 1) return 2;
  if (mode === 2) return 4;
  // big-integer mode
  const len = (bytes[offset]! >> 2) + 4;
  return 1 + len;
}

// Decode the value of a SCALE compact-encoded integer at the given offset.
function compactValue(bytes: Uint8Array, offset: number): number {
  const mode = bytes[offset]! & 0x03;
  if (mode === 0) return bytes[offset]! >> 2;
  if (mode === 1) return ((bytes[offset + 1]! << 8) | bytes[offset]!) >> 2;
  if (mode === 2) {
    const val = (bytes[offset + 3]! << 24) | (bytes[offset + 2]! << 16)
      | (bytes[offset + 1]! << 8) | bytes[offset]!;
    return val >>> 2;
  }
  // big-integer: not expected for extension lengths
  return 0;
}

// Extract call data from a SCALE-encoded extrinsic.
// For unsigned/bare: unambiguous — call data follows the version byte directly.
// For signed: returns null (extensions are runtime-specific, caller uses fallback).
function extractCallData(hexExt: HexString): Uint8Array | null {
  try {
    const bytes = Binary.fromHex(hexExt).asBytes();
    if (bytes.length < 3) return null;

    let offset = compactLen(bytes, 0);
    const preamble = bytes[offset]!;
    offset += 1;

    const version = preamble & 0x1f;

    if (version === 5) {
      const preambleType = (preamble >> 5) & 0x03;
      if (preambleType === 0) return bytes.slice(offset); // Bare
      if (preambleType === 2) {
        // General (0x45): compact(ext_len) ++ extension_bytes ++ call_data
        const extLenSize = compactLen(bytes, offset);
        const extLen = compactValue(bytes, offset);
        offset += extLenSize + extLen;
        return bytes.slice(offset);
      }
      return null; // Signed v5
    }

    if (version === 4) {
      if ((preamble & 0x80) === 0) return bytes.slice(offset); // Unsigned
      return null; // Signed — handled by caller via fallback
    }

    return null;
  } catch {
    return null;
  }
}

// For signed extrinsics: skip address + signature, then try offsets from the
// most likely position outward until txFromCallData succeeds.
function getSignedExtOffsetRange(hexExt: HexString): { bytes: Uint8Array; minOffset: number } | null {
  try {
    const bytes = Binary.fromHex(hexExt).asBytes();
    if (bytes.length < 3) return null;

    let offset = compactLen(bytes, 0);
    const preamble = bytes[offset]!;
    offset += 1;

    const version = preamble & 0x1f;
    const isSigned = version === 4 ? (preamble & 0x80) !== 0 : ((preamble >> 5) & 0x03) !== 0;
    if (!isSigned) return null;

    // Skip address
    const addrType = bytes[offset]!;
    offset += 1;
    if (addrType === 0) offset += 32;
    else return null;

    // Skip signature
    const sigType = bytes[offset]!;
    offset += 1 + (sigType === 2 ? 65 : 64);

    return { bytes, minOffset: offset };
  } catch {
    return null;
  }
}

export function Explorer() {
  const api = useApi();
  const client = useClient();
  const currentBlockNumber = useBlockNumber();
  const { storageType } = useChainState();

  const [selectedBlockNumber, setSelectedBlockNumber] = useState<number | null>(null);
  const [blockSearchInput, setBlockSearchInput] = useState("");

  const [recentBlocks, setRecentBlocks] = useState<BlockInfo[]>([]);
  const [isLoadingBlocks, setIsLoadingBlocks] = useState(false);

  const [selectedBlock, setSelectedBlock] = useState<BlockInfo | null>(null);
  const [blockExtrinsics, setBlockExtrinsics] = useState<ExtrinsicInfo[]>([]);
  const [isLoadingBlock, setIsLoadingBlock] = useState(false);

  // Load recent blocks using the client's block data
  const loadRecentBlocks = useCallback(async () => {
    if (!client || currentBlockNumber === undefined) return;

    setIsLoadingBlocks(true);
    try {
      const bestBlocks = await client.getBestBlocks();
      const blocks: BlockInfo[] = await Promise.all(
        bestBlocks.map(async (b) => {
          try {
            const body = await client.getBlockBody(b.hash);
            return {
              number: b.number,
              hash: b.hash,
                extrinsicsCount: body.length,
            };
          } catch {
            return {
              number: b.number,
              hash: b.hash,
                extrinsicsCount: 0,
            };
          }
        })
      );

      setRecentBlocks(blocks);
    } catch (err) {
      console.error("Failed to load recent blocks:", err);
    } finally {
      setIsLoadingBlocks(false);
    }
  }, [client, currentBlockNumber]);

  // Load block details. If knownHash is provided (e.g. from bestBlocks), use it
  // directly instead of querying System.BlockHash.
  const loadBlockDetails = useCallback(async (blockNumber: number, knownHash?: string) => {
    if (!client) return;

    setIsLoadingBlock(true);
    setSelectedBlockNumber(blockNumber);
    setBlockExtrinsics([]);

    try {
      // Use known hash or look it up from the recent blocks list
      let hashHex = knownHash
        ?? recentBlocks.find((b) => b.number === blockNumber)?.hash
        ?? "";

      // Fall back to System.BlockHash storage query
      if (!hashHex && api) {
        const blockHash = await api.query.System.BlockHash.getValue(blockNumber);
        if (blockHash) {
          const hex = blockHash.asHex();
          // Ignore zero hash (block not in storage)
          if (hex !== "0x0000000000000000000000000000000000000000000000000000000000000000") {
            hashHex = hex;
          }
        }
      }

      // Try to get block body - may fail for blocks not pinned by chainHead
      let body: string[] = [];
      try {
        if (hashHex) {
          body = await client.getBlockBody(hashHex);
        }
      } catch {
        // Block body not available (e.g. unpinned finalized block)
      }

      setSelectedBlock({
        number: blockNumber,
        hash: hashHex || "",
        extrinsicsCount: body.length,
      });

      // Decode each extrinsic to extract pallet + call name
      const extrinsics: ExtrinsicInfo[] = await Promise.all(
        body.map(async (hex, index) => {
          if (!api) return { index, pallet: "", call: "" };
          try {
            // Try direct extraction (works for unsigned/bare/general)
            const callData = extractCallData(hex as HexString);
            if (callData) {
              const tx = await api.txFromCallData(Binary.fromBytes(callData));
              const pallet = tx.decodedCall.type;
              const call = (tx.decodedCall.value as { type: string }).type;
              return { index, pallet, call };
            }

            // Signed extrinsic: try offsets after the signature, starting from
            // the end (shortest call data first to avoid false positives)
            const range = getSignedExtOffsetRange(hex as HexString);
            if (range) {
              for (let i = range.bytes.length - 2; i >= range.minOffset; i--) {
                try {
                  const slice = range.bytes.slice(i);
                  const tx = await api.txFromCallData(Binary.fromBytes(slice));
                  const pallet = tx.decodedCall.type;
                  const call = (tx.decodedCall.value as { type: string }).type;
                  return { index, pallet, call };
                } catch {
                  // Try next offset
                }
              }
            }
          } catch (e) {
            console.error(`[ext ${index}] decoding failed:`, e);
          }
          return { index, pallet: "", call: "" };
        })
      );
      setBlockExtrinsics(extrinsics);
    } catch (err) {
      console.error("Failed to load block details:", err);
      setSelectedBlock({
        number: blockNumber,
        hash: "",
        extrinsicsCount: 0,
      });
    } finally {
      setIsLoadingBlock(false);
    }
  }, [api, client, recentBlocks]);

  // Reset and reload when the network/client changes
  useEffect(() => {
    setRecentBlocks([]);
    setSelectedBlock(null);
    setSelectedBlockNumber(null);
    setBlockExtrinsics([]);
    setBlockSearchInput("");
  }, [api]);

  // Refresh recent blocks whenever the block number changes
  useEffect(() => {
    if (currentBlockNumber !== undefined) {
      loadRecentBlocks();
    }
  }, [currentBlockNumber, loadRecentBlocks]);

  // Auto-select the first block when blocks are loaded but none is selected
  useEffect(() => {
    if (recentBlocks.length > 0 && selectedBlockNumber === null) {
      loadBlockDetails(recentBlocks[0]!.number, recentBlocks[0]!.hash);
    }
  }, [recentBlocks, selectedBlockNumber, loadBlockDetails]);

  const handleBlockSearch = () => {
    const blockNum = parseInt(blockSearchInput);
    if (!isNaN(blockNum) && blockNum >= 0) {
      loadBlockDetails(blockNum);
    }
  };

  const handlePrevBlock = () => {
    if (selectedBlockNumber !== null && selectedBlockNumber > 0) {
      loadBlockDetails(selectedBlockNumber - 1);
    }
  };

  const handleNextBlock = () => {
    if (selectedBlockNumber !== null && currentBlockNumber !== undefined && selectedBlockNumber < currentBlockNumber) {
      loadBlockDetails(selectedBlockNumber + 1);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">
          {storageType === "web3storage" ? "Web3 Storage Explorer" : "Block Explorer"}
        </h1>
        <p className="text-muted-foreground">
          {storageType === "web3storage"
            ? "Browse blocks and storage transactions on Web3 Storage"
            : "Browse blocks and storage transactions on the Bulletin Chain"}
        </p>
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        {/* Recent Blocks */}
        <div className="space-y-4">
          <Card>
            <CardHeader className="pb-3">
              <div className="flex items-center justify-between">
                <CardTitle className="flex items-center gap-2 text-lg">
                  <Box className="h-5 w-5" />
                  Recent Blocks
                </CardTitle>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={loadRecentBlocks}
                  disabled={isLoadingBlocks}
                >
                  <RefreshCw className={`h-4 w-4 ${isLoadingBlocks ? "animate-spin" : ""}`} />
                </Button>
              </div>
            </CardHeader>
            <CardContent>
              {isLoadingBlocks && recentBlocks.length === 0 ? (
                <div className="flex justify-center py-8">
                  <Spinner />
                </div>
              ) : recentBlocks.length === 0 ? (
                <p className="text-center text-muted-foreground py-8">
                  No blocks loaded
                </p>
              ) : (
                <div className="space-y-2">
                  {recentBlocks.map((block) => (
                    <button
                      key={block.number}
                      onClick={() => loadBlockDetails(block.number, block.hash)}
                      className={`w-full text-left p-3 rounded-md transition-colors ${
                        selectedBlockNumber === block.number
                          ? "bg-primary/10 border border-primary/20"
                          : "hover:bg-secondary"
                      }`}
                    >
                      <div className="flex items-center justify-between">
                        <span className="font-mono font-medium">
                          #{block.number.toLocaleString()}
                        </span>
                        <Badge variant="secondary" className="text-xs">
                          {block.extrinsicsCount} txs
                        </Badge>
                      </div>
                    </button>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>

          {/* Block Search */}
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="flex items-center gap-2 text-lg">
                <Search className="h-5 w-5" />
                Search Block
              </CardTitle>
            </CardHeader>
            <CardContent>
              <div className="flex gap-2">
                <Input
                  type="number"
                  placeholder="Block number"
                  value={blockSearchInput}
                  onChange={(e) => setBlockSearchInput(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && handleBlockSearch()}
                  min={0}
                />
                <Button onClick={handleBlockSearch} disabled={!blockSearchInput}>
                  Go
                </Button>
              </div>
            </CardContent>
          </Card>
        </div>

        {/* Block Details */}
        <div className="lg:col-span-2">
          {isLoadingBlock ? (
            <Card>
              <CardContent className="flex justify-center py-16">
                <Spinner size="lg" />
              </CardContent>
            </Card>
          ) : selectedBlock ? (
            <div className="space-y-4">
              <Card>
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <div>
                      <CardTitle className="flex items-center gap-2">
                        <Hash className="h-5 w-5" />
                        Block {formatBlockNumber(selectedBlock.number)}
                      </CardTitle>
                      <CardDescription className="font-mono text-xs mt-1">
                        {selectedBlock.hash}
                      </CardDescription>
                    </div>
                    <div className="flex gap-1">
                      <Button
                        variant="outline"
                        size="icon"
                        onClick={handlePrevBlock}
                        disabled={selectedBlockNumber === 0}
                      >
                        <ChevronLeft className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="outline"
                        size="icon"
                        onClick={handleNextBlock}
                        disabled={currentBlockNumber !== undefined && selectedBlockNumber === currentBlockNumber}
                      >
                        <ChevronRight className="h-4 w-4" />
                      </Button>
                    </div>
                  </div>
                </CardHeader>
                <CardContent>
                  <div className="space-y-1">
                    <p className="text-xs text-muted-foreground uppercase tracking-wide">
                      Transactions
                    </p>
                    <p className="font-mono text-sm">
                      {blockExtrinsics.length}
                    </p>
                  </div>
                </CardContent>
              </Card>

              <Card>
                <CardHeader>
                  <CardTitle className="text-lg">Extrinsics</CardTitle>
                  <CardDescription>
                    {blockExtrinsics.length} extrinsic(s) in this block
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  {blockExtrinsics.length === 0 ? (
                    <p className="text-center text-muted-foreground py-4">
                      No extrinsics in this block
                    </p>
                  ) : (
                    <div className="space-y-2">
                      {blockExtrinsics.map((ext) => (
                        <div
                          key={ext.index}
                          className="p-3 rounded-md bg-secondary/50 border"
                        >
                          <div className="flex items-center gap-2">
                            <Badge variant="outline">#{ext.index}</Badge>
                            <span className="font-mono text-sm">
                              {ext.pallet && ext.call
                                ? `${ext.pallet}.${ext.call}`
                                : "Extrinsic"}
                            </span>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </CardContent>
              </Card>
            </div>
          ) : (
            <Card>
              <CardContent className="flex flex-col items-center justify-center py-16 text-muted-foreground">
                <Box className="h-12 w-12 mb-4" />
                <p>Select a block to view details</p>
              </CardContent>
            </Card>
          )}
        </div>
      </div>
    </div>
  );
}
