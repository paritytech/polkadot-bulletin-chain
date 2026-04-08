import { useState, useCallback, useEffect } from "react";
import { useSearchParams } from "react-router-dom";
import { RefreshCw, AlertCircle, Check, Clock, Database, Search, History, Info } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
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
import { AuthorizationCard } from "@/components/AuthorizationCard";
import { CidInput } from "@/components/CidInput";
import { useApi, useBlockNumber, useChainState, useCreateBulletinClient, useNetwork } from "@/state/chain.state";
import { useSelectedAccount } from "@/state/wallet.state";
import { fetchTransactionInfo, TransactionInfo } from "@/state/storage.state";
import { useStorageHistory } from "@/state/history.state";
import { formatBytes, bytesToHex } from "@/utils/format";
import { CID, WaitFor, UnixFsDagBuilder } from "@parity/bulletin-sdk";
import { useProgressHandler } from "@/hooks/useProgressHandler";
import { isDagPb, resolveAllCids, type CidResolution } from "@/lib/cid-lookup";
import { fetchRawBlock, IPFS_GATEWAYS } from "@/lib/ipfs";

interface RenewalTarget {
  blockNumber: number;
  index: number;
  info: TransactionInfo;
  expiresAtBlock: number;
}

interface BatchRenewResult {
  cidString: string;
  success: boolean;
  error?: string;
  newExpiresAt?: number;
}

export function Renew() {
  const api = useApi();
  const createBulletinClient = useCreateBulletinClient();
  const { network } = useChainState();
  const currentNetwork = useNetwork();
  const selectedAccount = useSelectedAccount();
  const currentBlockNumber = useBlockNumber();
  const [searchParams, setSearchParams] = useSearchParams();
  const allHistory = useStorageHistory();

  // Filter history for current network
  const networkHistory = allHistory.filter((e) => e.networkId === network.id);

  // Tab state
  const [activeTab, setActiveTab] = useState<string>("block-index");

  // Form inputs (block+index tab)
  const [blockInput, setBlockInput] = useState("");
  const [indexInput, setIndexInput] = useState("");

  // Lookup state (block+index tab)
  const [isLookingUp, setIsLookingUp] = useState(false);
  const [lookupError, setLookupError] = useState<string | null>(null);
  const [renewalTarget, setRenewalTarget] = useState<RenewalTarget | null>(null);

  // Renewal state (block+index tab)
  const [isRenewing, setIsRenewing] = useState(false);
  const [renewalError, setRenewalError] = useState<string | null>(null);
  const [renewalSuccess, setRenewalSuccess] = useState<{
    blockNumber?: number;
    newExpiresAt: number;
  } | null>(null);
  const [txStatus, setTxStatus] = useState<string | null>(null);
  const handleProgress = useProgressHandler(setTxStatus);

  // CID input state (by-cid tab)
  const [cidInput, setCidInput] = useState("");
  const [isCidValid, setIsCidValid] = useState(false);
  const [parsedCid, setParsedCid] = useState<CID | undefined>();

  // CID resolution state
  const [isResolving, setIsResolving] = useState(false);
  const [resolveError, setResolveError] = useState<string | null>(null);
  const [resolveProgress, setResolveProgress] = useState<string | null>(null);
  const [resolutions, setResolutions] = useState<CidResolution[]>([]);
  const [checkedCids, setCheckedCids] = useState<Set<string>>(new Set());

  // Batch renewal state
  const [isBatchRenewing, setIsBatchRenewing] = useState(false);
  const [batchProgress, setBatchProgress] = useState<string | null>(null);
  const [batchResults, setBatchResults] = useState<BatchRenewResult[]>([]);

  // Retention period from chain
  const [retentionPeriod, setRetentionPeriod] = useState<number | null>(null);

  // Fetch retention period on mount
  useEffect(() => {
    async function fetchRetentionPeriod() {
      if (!api) return;
      try {
        const period = await api.query.TransactionStorage.RetentionPeriod.getValue();
        setRetentionPeriod(Number(period));
      } catch (err) {
        console.error("Failed to fetch retention period:", err);
      }
    }
    fetchRetentionPeriod();
  }, [api]);

  // Load from URL params on mount
  useEffect(() => {
    const blockParam = searchParams.get("block");
    const indexParam = searchParams.get("index");
    if (blockParam && indexParam) {
      setBlockInput(blockParam);
      setIndexInput(indexParam);
      // Clear params after loading
      setSearchParams({}, { replace: true });
    }
  }, [searchParams, setSearchParams]);

  // Handle history selection
  const handleHistorySelect = (value: string) => {
    if (value === "none") return;
    const parts = value.split("-");
    const block = parseInt(parts[0] ?? "0", 10);
    const index = parseInt(parts[1] ?? "0", 10);
    setBlockInput(block.toString());
    setIndexInput(index.toString());
  };

  const handleLookup = useCallback(async () => {
    if (!api) return;

    const blockNum = parseInt(blockInput);
    const idx = parseInt(indexInput);

    if (isNaN(blockNum) || blockNum < 0) {
      setLookupError("Please enter a valid block number");
      return;
    }
    if (isNaN(idx) || idx < 0) {
      setLookupError("Please enter a valid transaction index");
      return;
    }

    setIsLookingUp(true);
    setLookupError(null);
    setRenewalTarget(null);
    setRenewalSuccess(null);
    setRenewalError(null);

    try {
      const info = await fetchTransactionInfo(api, blockNum, idx);

      if (!info) {
        setLookupError(`No storage transaction found at block ${blockNum}, index ${idx}`);
        return;
      }

      // retentionPeriod is guaranteed non-null here (lookup button is disabled until loaded)
      const expiresAtBlock = blockNum + retentionPeriod!;

      setRenewalTarget({
        blockNumber: blockNum,
        index: idx,
        info,
        expiresAtBlock,
      });
    } catch (err) {
      console.error("Lookup failed:", err);
      setLookupError(err instanceof Error ? err.message : "Failed to lookup transaction");
    } finally {
      setIsLookingUp(false);
    }
  }, [api, blockInput, indexInput, retentionPeriod]);

  const handleRenew = useCallback(async () => {
    if (!api || !selectedAccount?.polkadotSigner || !renewalTarget) return;

    setIsRenewing(true);
    setRenewalError(null);
    setRenewalSuccess(null);
    setTxStatus(null);

    try {
      // Create SDK client with user's signer
      const bulletinClient = createBulletinClient!(selectedAccount.polkadotSigner);

      // Use SDK to renew with progress callback
      const result = await bulletinClient
        .renew(renewalTarget.blockNumber, renewalTarget.index)
        .withCallback(handleProgress)
        .withWaitFor(WaitFor.Finalized)
        .send();

      // Calculate new expiration (retentionPeriod guaranteed non-null at this point)
      const renewedAtBlock = result.blockNumber ?? (currentBlockNumber ?? 0);
      const newExpiresAt = renewedAtBlock + retentionPeriod!;

      setRenewalSuccess({
        blockNumber: result.blockNumber,
        newExpiresAt,
      });

      // Clear the target after successful renewal
      setRenewalTarget(null);
    } catch (err) {
      console.error("Renewal failed:", err);
      setRenewalError(err instanceof Error ? err.message : "Renewal failed");
    } finally {
      setIsRenewing(false);
      setTxStatus(null);
    }
  }, [api, selectedAccount, renewalTarget, currentBlockNumber, retentionPeriod]);

  // CID input handler
  const handleCidChange = (value: string, isValid: boolean, cid?: CID) => {
    setCidInput(value);
    setIsCidValid(isValid);
    setParsedCid(cid);
    // Clear previous resolution when CID changes
    setResolutions([]);
    setCheckedCids(new Set());
    setResolveError(null);
    setBatchResults([]);
  };

  // Resolve CID to on-chain locations
  const handleResolveCid = useCallback(async () => {
    if (!api || !parsedCid) return;

    setIsResolving(true);
    setResolveError(null);
    setResolveProgress(null);
    setResolutions([]);
    setCheckedCids(new Set());
    setBatchResults([]);

    try {
      let cidsToResolve: { cid: CID; isManifest: boolean }[];

      if (isDagPb(parsedCid)) {
        // Fetch and parse DAG-PB manifest
        const gatewayUrl = IPFS_GATEWAYS[currentNetwork.id];
        if (!gatewayUrl) {
          setResolveError(
            `No IPFS gateway configured for the "${currentNetwork.id}" network. ` +
            `Use the "By Block + Index" tab to renew manually.`
          );
          return;
        }

        setResolveProgress("Fetching DAG-PB manifest...");
        const dagBytes = await fetchRawBlock(parsedCid.toString(), gatewayUrl);

        setResolveProgress("Parsing manifest...");
        const dagBuilder = new UnixFsDagBuilder();
        const { chunkCids } = await dagBuilder.parse(dagBytes);

        // Root manifest + all child chunks
        cidsToResolve = [
          { cid: parsedCid, isManifest: true },
          ...chunkCids.map((c: CID) => ({ cid: c, isManifest: false })),
        ];
      } else {
        // Single raw CID
        cidsToResolve = [{ cid: parsedCid, isManifest: false }];
      }

      setResolveProgress(`Resolving ${cidsToResolve.length} CID(s) on chain...`);

      const resolved = await resolveAllCids(
        api,
        cidsToResolve,
        networkHistory,
        (done, total) => {
          setResolveProgress(`Resolving CIDs: ${done} / ${total}`);
        },
      );

      setResolutions(resolved);

      // Check all found CIDs by default
      const foundCids = new Set(
        resolved.filter((r) => r.found).map((r) => r.cidString),
      );
      setCheckedCids(foundCids);

      if (resolved.every((r) => !r.found)) {
        setResolveError("None of the CIDs were found on chain. The data may have expired or was never stored on this network.");
      }
    } catch (err) {
      console.error("CID resolution failed:", err);
      setResolveError(err instanceof Error ? err.message : "Failed to resolve CID");
    } finally {
      setIsResolving(false);
      setResolveProgress(null);
    }
  }, [api, parsedCid, currentNetwork.id, networkHistory]);

  // Toggle a single CID checkbox
  const handleToggleCid = (cidString: string) => {
    setCheckedCids((prev) => {
      const next = new Set(prev);
      if (next.has(cidString)) {
        next.delete(cidString);
      } else {
        next.add(cidString);
      }
      return next;
    });
  };

  const handleSelectAll = () => {
    setCheckedCids(new Set(resolutions.filter((r) => r.found).map((r) => r.cidString)));
  };

  const handleDeselectAll = () => {
    setCheckedCids(new Set());
  };

  // Batch renew selected CIDs
  const handleBatchRenew = useCallback(async () => {
    if (!api || !selectedAccount?.polkadotSigner) return;

    const toRenew = resolutions.filter(
      (r) => r.found && checkedCids.has(r.cidString) && r.blockNumber !== null && r.index !== null,
    );

    if (toRenew.length === 0) return;

    setIsBatchRenewing(true);
    setBatchProgress(null);
    setBatchResults([]);

    const bulletinClient = createBulletinClient!(selectedAccount.polkadotSigner);
    const results: BatchRenewResult[] = [];

    for (let i = 0; i < toRenew.length; i++) {
      const resolution = toRenew[i]!;
      const cidStr = resolution.cidString;
      const shortCid = cidStr.length > 20 ? `${cidStr.slice(0, 10)}...${cidStr.slice(-6)}` : cidStr;
      setBatchProgress(`Renewing ${i + 1} of ${toRenew.length}: ${shortCid}`);

      try {
        const result = await bulletinClient
          .renew(resolution.blockNumber!, resolution.index!)
          .withCallback(handleProgress)
          .withWaitFor(WaitFor.Finalized)
          .send();

        const renewedAtBlock = result.blockNumber ?? (currentBlockNumber ?? 0);
        const newExpiresAt = renewedAtBlock + (retentionPeriod ?? 0);

        results.push({ cidString: cidStr, success: true, newExpiresAt });
      } catch (err) {
        console.error(`Failed to renew ${cidStr}:`, err);
        results.push({
          cidString: cidStr,
          success: false,
          error: err instanceof Error ? err.message : "Renewal failed",
        });
      }
    }

    setBatchResults(results);
    setIsBatchRenewing(false);
    setBatchProgress(null);
    setTxStatus(null);
  }, [api, selectedAccount, resolutions, checkedCids, currentBlockNumber, retentionPeriod]);

  const canRenew =
    api &&
    selectedAccount?.polkadotSigner &&
    renewalTarget &&
    !isRenewing;

  // Calculate blocks until expiration
  const blocksUntilExpiration = renewalTarget && currentBlockNumber !== undefined
    ? renewalTarget.expiresAtBlock - currentBlockNumber
    : null;

  const isExpired = blocksUntilExpiration !== null && blocksUntilExpiration <= 0;
  const isExpiringSoon = blocksUntilExpiration !== null && blocksUntilExpiration > 0 && blocksUntilExpiration < 1000;

  // CID tab helpers
  const checkedCount = resolutions.filter((r) => r.found && checkedCids.has(r.cidString)).length;
  const foundCount = resolutions.filter((r) => r.found).length;
  const canBatchRenew = api && selectedAccount?.polkadotSigner && checkedCount > 0 && !isBatchRenewing;

  // Calculate expiration for a resolution
  const getExpirationInfo = (resolution: CidResolution) => {
    if (!resolution.found || resolution.blockNumber === null || retentionPeriod === null || currentBlockNumber === undefined) {
      return null;
    }
    const expiresAt = resolution.blockNumber + retentionPeriod;
    const remaining = expiresAt - currentBlockNumber;
    return { expiresAt, remaining, expired: remaining <= 0, expiringSoon: remaining > 0 && remaining < 1000 };
  };

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Renew Storage</h1>
        <p className="text-muted-foreground">
          Extend the retention period for your stored data
        </p>
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2 space-y-6">
          {/* Lookup Card */}
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Search className="h-5 w-5" />
                Find Storage Transaction
              </CardTitle>
              <CardDescription>
                Look up stored data by block number and index, or resolve a CID
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Tabs value={activeTab} onValueChange={setActiveTab}>
                <TabsList className="mb-4">
                  <TabsTrigger value="block-index">By Block + Index</TabsTrigger>
                  <TabsTrigger value="by-cid">By CID</TabsTrigger>
                </TabsList>

                {/* Tab 1: Block + Index (existing flow) */}
                <TabsContent value="block-index" className="space-y-4">
                  {/* History Selector */}
                  {networkHistory.length > 0 && (
                    <div className="space-y-2">
                      <label className="text-sm font-medium flex items-center gap-2">
                        <History className="h-4 w-4" />
                        Load from History
                      </label>
                      <Select onValueChange={handleHistorySelect}>
                        <SelectTrigger>
                          <SelectValue placeholder="Select a previous upload..." />
                        </SelectTrigger>
                        <SelectContent>
                          {networkHistory.map((entry) => (
                            <SelectItem
                              key={`${entry.blockNumber}-${entry.index}`}
                              value={`${entry.blockNumber}-${entry.index}`}
                            >
                              <div className="flex items-center gap-2">
                                <span className="font-mono text-xs">
                                  Block #{entry.blockNumber}
                                </span>
                                {entry.label && (
                                  <span className="text-muted-foreground">
                                    - {entry.label}
                                  </span>
                                )}
                                <Badge variant="secondary" className="text-xs">
                                  {formatBytes(entry.size)}
                                </Badge>
                              </div>
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                  )}

                  {/* Manual Entry */}
                  <div className="grid sm:grid-cols-2 gap-4">
                    <div className="space-y-2">
                      <label className="text-sm font-medium">Block Number</label>
                      <Input
                        type="number"
                        placeholder="e.g., 12345"
                        value={blockInput}
                        onChange={(e) => setBlockInput(e.target.value)}
                        onKeyDown={(e) => e.key === "Enter" && handleLookup()}
                        min={0}
                        disabled={isLookingUp}
                      />
                    </div>
                    <div className="space-y-2">
                      <label className="text-sm font-medium">Transaction Index</label>
                      <Input
                        type="number"
                        placeholder="e.g., 0"
                        value={indexInput}
                        onChange={(e) => setIndexInput(e.target.value)}
                        onKeyDown={(e) => e.key === "Enter" && handleLookup()}
                        min={0}
                        disabled={isLookingUp}
                      />
                    </div>
                  </div>

                  <Button
                    onClick={handleLookup}
                    disabled={!api || isLookingUp || !blockInput || !indexInput || retentionPeriod === null}
                    className="w-full"
                  >
                    {isLookingUp ? (
                      <>
                        <Spinner size="sm" className="mr-2" />
                        Looking up...
                      </>
                    ) : (
                      <>
                        <Search className="h-4 w-4 mr-2" />
                        Lookup Transaction
                      </>
                    )}
                  </Button>

                  {lookupError && (
                    <div className="flex items-start gap-3 p-3 rounded-md bg-destructive/10 text-destructive">
                      <AlertCircle className="h-5 w-5 mt-0.5" />
                      <p className="text-sm">{lookupError}</p>
                    </div>
                  )}
                </TabsContent>

                {/* Tab 2: By CID */}
                <TabsContent value="by-cid" className="space-y-4">
                  <div className="space-y-2">
                    <label className="text-sm font-medium">CID</label>
                    <CidInput
                      value={cidInput}
                      onChange={handleCidChange}
                      disabled={isResolving}
                    />
                  </div>

                  {parsedCid && isDagPb(parsedCid) && (
                    <div className="flex items-center gap-2 p-2 rounded-md bg-blue-500/10 text-blue-600 dark:text-blue-400">
                      <Info className="h-4 w-4" />
                      <span className="text-sm">
                        DAG-PB manifest detected. Child chunk CIDs will be resolved automatically.
                      </span>
                    </div>
                  )}

                  <Button
                    onClick={handleResolveCid}
                    disabled={!api || !isCidValid || isResolving || retentionPeriod === null}
                    className="w-full"
                  >
                    {isResolving ? (
                      <>
                        <Spinner size="sm" className="mr-2" />
                        {resolveProgress || "Resolving..."}
                      </>
                    ) : (
                      <>
                        <Search className="h-4 w-4 mr-2" />
                        Resolve CID
                      </>
                    )}
                  </Button>

                  {resolveError && (
                    <div className="flex items-start gap-3 p-3 rounded-md bg-destructive/10 text-destructive">
                      <AlertCircle className="h-5 w-5 mt-0.5" />
                      <p className="text-sm">{resolveError}</p>
                    </div>
                  )}

                  {/* Resolution Results */}
                  {resolutions.length > 0 && (
                    <div className="space-y-3">
                      <div className="flex items-center justify-between">
                        <span className="text-sm text-muted-foreground">
                          Found {foundCount} of {resolutions.length} CID(s) on chain
                        </span>
                        <div className="flex gap-1">
                          <Button variant="ghost" size="sm" onClick={handleSelectAll}>
                            Select All
                          </Button>
                          <Button variant="ghost" size="sm" onClick={handleDeselectAll}>
                            Deselect All
                          </Button>
                        </div>
                      </div>

                      <div className="space-y-2 max-h-[400px] overflow-y-auto">
                        {resolutions.map((r, i) => {
                          const expInfo = getExpirationInfo(r);
                          const shortCid = r.cidString.length > 30
                            ? `${r.cidString.slice(0, 14)}...${r.cidString.slice(-8)}`
                            : r.cidString;

                          return (
                            <div
                              key={r.cidString}
                              className="flex items-center gap-3 p-3 rounded-md border bg-card"
                            >
                              <input
                                type="checkbox"
                                checked={checkedCids.has(r.cidString)}
                                disabled={!r.found}
                                onChange={() => handleToggleCid(r.cidString)}
                                className="h-4 w-4 rounded border-input accent-primary"
                              />
                              <div className="flex-1 min-w-0 space-y-1">
                                <div className="flex items-center gap-2 flex-wrap">
                                  <span className="font-mono text-xs truncate" title={r.cidString}>
                                    {shortCid}
                                  </span>
                                  <Badge variant="secondary" className="text-xs">
                                    {r.isManifest ? "Manifest" : `Chunk ${i}`}
                                  </Badge>
                                </div>
                                <div className="flex items-center gap-2 flex-wrap">
                                  {r.found ? (
                                    <>
                                      <span className="text-xs text-muted-foreground">
                                        Block #{r.blockNumber}, Index #{r.index}
                                      </span>
                                      {expInfo && (
                                        <>
                                          {expInfo.expired ? (
                                            <Badge variant="destructive" className="text-xs">Expired</Badge>
                                          ) : expInfo.expiringSoon ? (
                                            <Badge className="bg-yellow-500 text-xs">Expiring Soon</Badge>
                                          ) : (
                                            <Badge variant="secondary" className="text-xs">Active</Badge>
                                          )}
                                        </>
                                      )}
                                    </>
                                  ) : (
                                    <span className="text-xs text-destructive">Not found on chain</span>
                                  )}
                                </div>
                              </div>
                            </div>
                          );
                        })}
                      </div>

                      {/* Batch Renew Button */}
                      <Button
                        onClick={handleBatchRenew}
                        disabled={!canBatchRenew}
                        className="w-full"
                        size="lg"
                      >
                        {isBatchRenewing ? (
                          <>
                            <Spinner size="sm" className="mr-2" />
                            {batchProgress || txStatus || "Renewing..."}
                          </>
                        ) : (
                          <>
                            <RefreshCw className="h-5 w-5 mr-2" />
                            Renew Selected ({checkedCount})
                          </>
                        )}
                      </Button>

                      {!selectedAccount && (
                        <p className="text-sm text-muted-foreground text-center">
                          Connect a wallet to renew data
                        </p>
                      )}
                    </div>
                  )}

                  {/* Batch Results */}
                  {batchResults.length > 0 && (
                    <div className="space-y-2">
                      <h4 className="text-sm font-medium">Renewal Results</h4>
                      {batchResults.map((r) => {
                        const shortCid = r.cidString.length > 30
                          ? `${r.cidString.slice(0, 14)}...${r.cidString.slice(-8)}`
                          : r.cidString;
                        return (
                          <div
                            key={r.cidString}
                            className={`flex items-center gap-2 p-2 rounded-md text-sm ${
                              r.success
                                ? "bg-green-500/10 text-green-700 dark:text-green-400"
                                : "bg-destructive/10 text-destructive"
                            }`}
                          >
                            {r.success ? (
                              <Check className="h-4 w-4 shrink-0" />
                            ) : (
                              <AlertCircle className="h-4 w-4 shrink-0" />
                            )}
                            <span className="font-mono text-xs truncate">{shortCid}</span>
                            {r.success && r.newExpiresAt && (
                              <span className="text-xs ml-auto whitespace-nowrap">
                                Expires at #{r.newExpiresAt.toLocaleString()}
                              </span>
                            )}
                            {!r.success && r.error && (
                              <span className="text-xs ml-auto truncate">{r.error}</span>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  )}
                </TabsContent>
              </Tabs>
            </CardContent>
          </Card>

          {/* Transaction Info Card (block+index tab) */}
          {activeTab === "block-index" && renewalTarget && (
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <Database className="h-5 w-5" />
                  Storage Transaction
                </CardTitle>
                <CardDescription>
                  Block #{renewalTarget.blockNumber}, Index #{renewalTarget.index}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="grid sm:grid-cols-2 gap-4">
                  <div className="space-y-1">
                    <p className="text-xs text-muted-foreground uppercase tracking-wide">
                      Content Hash
                    </p>
                    <p className="font-mono text-xs break-all">
                      {bytesToHex(renewalTarget.info.contentHash)}
                    </p>
                  </div>
                  <div className="space-y-1">
                    <p className="text-xs text-muted-foreground uppercase tracking-wide">
                      Size
                    </p>
                    <p className="font-mono">
                      {formatBytes(renewalTarget.info.size)}
                    </p>
                  </div>
                </div>

                <div className="p-4 rounded-md bg-secondary/50 border">
                  <div className="flex items-center gap-2 mb-2">
                    <Clock className="h-4 w-4 text-muted-foreground" />
                    <span className="text-sm font-medium">Expiration Status</span>
                  </div>
                  <div className="flex items-center gap-2">
                    {isExpired ? (
                      <Badge variant="destructive">Expired</Badge>
                    ) : isExpiringSoon ? (
                      <Badge className="bg-yellow-500">Expiring Soon</Badge>
                    ) : (
                      <Badge variant="secondary">Active</Badge>
                    )}
                    {blocksUntilExpiration !== null && (
                      <span className="text-sm text-muted-foreground">
                        {isExpired
                          ? `Expired ${Math.abs(blocksUntilExpiration).toLocaleString()} blocks ago`
                          : `${blocksUntilExpiration.toLocaleString()} blocks remaining`}
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground mt-2">
                    Expires at block #{renewalTarget.expiresAtBlock.toLocaleString()}
                    {retentionPeriod && (
                      <> (Retention period: {retentionPeriod.toLocaleString()} blocks)</>
                    )}
                  </p>
                </div>

                <Button
                  onClick={handleRenew}
                  disabled={!canRenew}
                  className="w-full"
                  size="lg"
                >
                  {isRenewing ? (
                    <>
                      <Spinner size="sm" className="mr-2" />
                      {txStatus || "Renewing..."}
                    </>
                  ) : (
                    <>
                      <RefreshCw className="h-5 w-5 mr-2" />
                      Renew Storage
                    </>
                  )}
                </Button>

                {renewalError && (
                  <div className="flex items-start gap-3 p-3 rounded-md bg-destructive/10 text-destructive">
                    <AlertCircle className="h-5 w-5 mt-0.5" />
                    <div>
                      <p className="font-medium">Renewal Failed</p>
                      <p className="text-sm mt-1">{renewalError}</p>
                    </div>
                  </div>
                )}
              </CardContent>
            </Card>
          )}

          {/* Success Card (block+index tab) */}
          {activeTab === "block-index" && renewalSuccess && (
            <Card className="border-success">
              <CardHeader>
                <CardTitle className="flex items-center gap-2 text-success">
                  <Check className="h-5 w-5" />
                  Renewal Successful
                </CardTitle>
                <CardDescription>
                  Your data retention period has been extended
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="grid sm:grid-cols-2 gap-4">
                  {renewalSuccess.blockNumber && (
                    <div className="space-y-1">
                      <p className="text-xs text-muted-foreground uppercase tracking-wide">
                        Renewed in Block
                      </p>
                      <p className="font-mono">
                        #{renewalSuccess.blockNumber.toLocaleString()}
                      </p>
                    </div>
                  )}
                  <div className="space-y-1">
                    <p className="text-xs text-muted-foreground uppercase tracking-wide">
                      New Expiration Block
                    </p>
                    <p className="font-mono">
                      #{renewalSuccess.newExpiresAt.toLocaleString()}
                    </p>
                  </div>
                </div>
              </CardContent>
            </Card>
          )}

          {/* Info Card */}
          <Card>
            <CardHeader>
              <CardTitle className="text-lg">About Renewal</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3 text-sm text-muted-foreground">
              <p>
                Data stored on Bulletin Chain has a retention period. After this period,
                the data may be pruned from the network unless renewed.
              </p>
              <p>
                To renew your data, you need the <strong>block number</strong> and{" "}
                <strong>transaction index</strong> from when your data was originally stored.
                This information is provided when you upload data.
              </p>
              <p>
                You can also renew by <strong>CID</strong>. For DAG-PB (chunked) uploads, all
                child chunk CIDs will be automatically resolved and can be renewed in batch.
              </p>
              <p>
                Renewal extends the retention period from the current block, giving your
                data another full retention period before expiration.
              </p>
              {retentionPeriod && (
                <p>
                  <strong>Current retention period:</strong>{" "}
                  {retentionPeriod.toLocaleString()} blocks
                </p>
              )}
            </CardContent>
          </Card>
        </div>

        {/* Sidebar */}
        <div className="space-y-6">
          <AuthorizationCard />

          {!selectedAccount && (
            <Card>
              <CardContent className="pt-6">
                <div className="text-center text-muted-foreground">
                  <p className="mb-4">Connect a wallet to renew data</p>
                  <Button variant="outline" asChild>
                    <a href="/accounts">Connect Wallet</a>
                  </Button>
                </div>
              </CardContent>
            </Card>
          )}
        </div>
      </div>
    </div>
  );
}
