import { useState, useCallback, useEffect } from "react";
import { useSearchParams } from "react-router-dom";
import { RefreshCw, AlertCircle, Check, Clock, Database, Search, History } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select";
import { AuthorizationCard } from "@/components/AuthorizationCard";
import { useApi, useBlockNumber, useChainState, useCreateBulletinClient } from "@/state/chain.state";
import { useSelectedAccount } from "@/state/wallet.state";
import { fetchTransactionInfo, TransactionInfo } from "@/state/storage.state";
import { useStorageHistory } from "@/state/history.state";
import { formatBytes } from "@/utils/format";
import { bytesToHex, ProgressEvent } from "@bulletin/sdk";

interface RenewalTarget {
  blockNumber: number;
  index: number;
  info: TransactionInfo;
  expiresAtBlock: number;
}

export function Renew() {
  const api = useApi();
  const createBulletinClient = useCreateBulletinClient();
  const { network } = useChainState();
  const selectedAccount = useSelectedAccount();
  const currentBlockNumber = useBlockNumber();
  const [searchParams, setSearchParams] = useSearchParams();
  const allHistory = useStorageHistory();

  // Filter history for current network
  const networkHistory = allHistory.filter((e) => e.networkId === network.id);

  // Form inputs
  const [blockInput, setBlockInput] = useState("");
  const [indexInput, setIndexInput] = useState("");

  // Lookup state
  const [isLookingUp, setIsLookingUp] = useState(false);
  const [lookupError, setLookupError] = useState<string | null>(null);
  const [renewalTarget, setRenewalTarget] = useState<RenewalTarget | null>(null);

  // Renewal state
  const [isRenewing, setIsRenewing] = useState(false);
  const [renewalError, setRenewalError] = useState<string | null>(null);
  const [renewalSuccess, setRenewalSuccess] = useState<{
    blockNumber?: number;
    newExpiresAt: number;
  } | null>(null);
  const [txStatus, setTxStatus] = useState<string | null>(null);

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

      // Calculate expiration block
      const expiresAtBlock = retentionPeriod ? blockNum + retentionPeriod : blockNum;

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

      // Progress callback for transaction status updates
      const handleProgress = (event: ProgressEvent) => {
        console.log("SDK progress:", event);
        if (event.type === "signed") {
          setTxStatus("Transaction signed...");
        } else if (event.type === "broadcasted") {
          setTxStatus("Broadcasting to network...");
        } else if (event.type === "best_block") {
          setTxStatus(`Included in block #${event.blockNumber}...`);
        } else if (event.type === "finalized") {
          setTxStatus("Finalized!");
        }
      };

      // Use SDK to renew with progress callback
      const result = await bulletinClient.renew(
        renewalTarget.blockNumber,
        renewalTarget.index,
        handleProgress,
      );

      // Calculate new expiration
      const renewedAtBlock = result.blockNumber ?? (currentBlockNumber ?? 0);
      const newExpiresAt = retentionPeriod ? renewedAtBlock + retentionPeriod : renewedAtBlock;

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
                Select from your history or enter the block number and transaction index
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
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
                disabled={!api || isLookingUp || !blockInput || !indexInput}
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
            </CardContent>
          </Card>

          {/* Transaction Info Card */}
          {renewalTarget && (
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

          {/* Success Card */}
          {renewalSuccess && (
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
