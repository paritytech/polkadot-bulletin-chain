// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

import { useState, useCallback, useEffect } from "react";
import { useSearchParams } from "react-router-dom";
import type { HexString } from "polkadot-api";
import { RefreshCw, AlertCircle, Check, Clock, Copy, Database, Search, History, Info } from "lucide-react";
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
import { CidInfoCard } from "@/components/CidInfoCard";
import { useApi, useBlockNumber, useChainState, useCreateBulletinClient, useNetwork } from "@/state/chain.state";
import { useSelectedAccount } from "@/state/wallet.state";
import { fetchTransactionInfo, TransactionInfo } from "@/state/storage.state";
import { useStorageHistory } from "@/state/history.state";
import { formatBytes, bytesToHex, formatBlockDuration } from "@/utils/format";
import { cn } from "@/utils/cn";
import {
  BulletinError,
  CID,
  ErrorCode,
  WaitFor,
  type BulletinClientInterface,
  type ProgressCallback,
} from "@parity/bulletin-sdk";
import { useProgressHandler } from "@/hooks/useProgressHandler";
import {
  fetchRenewalRegistration,
  isDagPb,
  resolveCid,
  type CidResolution,
  type OnChainTransaction,
  type RenewalRegistration,
} from "@/lib/cid-lookup";
import { fetchRawBlock } from "@/lib/ipfs";

// The SDK rejects calls the live runtime lacks with UNSUPPORTED_OPERATION:
// forceRenew on pre-`TransactionRef` runtimes (where plain renew is already an
// immediate renewal), and the auto-renew calls on runtimes predating them.
function isUnsupportedOperation(err: unknown): boolean {
  return (
    err instanceof BulletinError &&
    err.code === ErrorCode.UNSUPPORTED_OPERATION
  );
}

/** ~1 day at 6s blocks; matches the Download page's on-chain status badge. */
const EXPIRING_SOON_BLOCKS = 14400;

/** Which renewal call a button triggers. */
type RenewAction = "scheduled" | "immediate" | "auto";

interface RenewTarget {
  block: number;
  index: number;
  contentHash: Uint8Array;
}

/**
 * Submit one renewal action.
 *
 * `immediate` falls back to plain `renew` on pre-`force_renew` runtimes, where
 * it is already immediate. The two registering actions get no fallback — a
 * legacy `renew` would renew now, which is not what was asked for.
 */
function submitRenewAction(
  client: BulletinClientInterface,
  action: RenewAction,
  target: RenewTarget,
  waitFor: WaitFor,
  onProgress: ProgressCallback,
) {
  const ref = { block: target.block, index: target.index };
  if (action === "auto") {
    return client
      .enableAutoRenew(target.contentHash)
      .withCallback(onProgress)
      .withWaitFor(waitFor)
      .send();
  }
  if (action === "scheduled") {
    return client.renew(ref).withCallback(onProgress).withWaitFor(waitFor).send();
  }
  return client
    .forceRenew(ref)
    .withCallback(onProgress)
    .withWaitFor(waitFor)
    .send()
    .catch((err) => {
      if (!isUnsupportedOperation(err)) throw err;
      return client.renew(ref).withCallback(onProgress).withWaitFor(waitFor).send();
    });
}

/** Auto-renew is absent on older runtimes; say so instead of leaking the SDK message. */
function renewErrorMessage(err: unknown, action: RenewAction): string {
  if (action === "auto" && isUnsupportedOperation(err)) {
    return "Auto-renew is not available on this network.";
  }
  return err instanceof Error ? err.message : "Renewal failed";
}

/** All three actions are feeless; the registering ones are prepaid at registration. */
function RenewActionLegend() {
  return (
    <div className="space-y-1 text-xs text-muted-foreground">
      <p>
        <strong className="text-foreground">Renew</strong> — schedules one renewal that
        fires when the current retention period ends. Retention does not change yet.
      </p>
      <p>
        <strong className="text-foreground">Renew immediately</strong> — renews now; the
        full retention period restarts from the current block.
      </p>
      <p>
        <strong className="text-foreground">Enable auto-renew</strong> — renews every
        retention period. The first cycle is prepaid; later cycles use your authorization
        quota and stop when it runs out.
      </p>
    </div>
  );
}

const BATCH_SUMMARY_VERBS: Record<RenewAction, string> = {
  scheduled: "Scheduled a renewal for",
  immediate: "Renewed",
  auto: "Enabled auto-renew for",
};

const SUCCESS_TITLES: Record<RenewAction, string> = {
  scheduled: "Renewal Scheduled",
  immediate: "Renewal Successful",
  auto: "Auto-Renew Enabled",
};

const SUCCESS_DESCRIPTIONS: Record<RenewAction, string> = {
  scheduled:
    "One renewal will fire when the current retention period ends. Retention has not changed yet.",
  immediate: "Your data retention period has been extended",
  auto: "This data will be renewed every retention period while your quota allows",
};

/**
 * Badge for an existing `Renewals` entry, with a disable action for recurring
 * ones. The chain refuses `disable_auto_renew` while the next cycle is still
 * prepaid, so the button waits for that cycle to fire rather than failing.
 */
function RegistrationBadge({
  registration,
  onDisable,
  busy,
}: {
  registration: RenewalRegistration;
  onDisable?: () => void;
  busy?: boolean;
}) {
  return (
    <>
      <Badge variant="secondary" className="text-xs">
        {registration.recurring ? "Auto-renew" : "Renewal scheduled"}
      </Badge>
      {registration.recurring && onDisable && (
        <Button
          variant="ghost"
          size="sm"
          onClick={onDisable}
          disabled={busy || registration.paid}
          title={
            registration.paid
              ? "Cannot disable until the prepaid cycle fires"
              : undefined
          }
        >
          Disable
        </Button>
      )}
    </>
  );
}

interface RenewalTarget {
  blockNumber: number;
  index: number;
  info: TransactionInfo;
  expiresAtBlock: number;
}

interface BatchRenewResult {
  cidString: string;
  success: boolean;
  /** Set only for `immediate`; the registering actions do not move the expiry yet. */
  newExpiresAt?: number;
  error?: string;
}

interface ResolveResults {
  resolutions: CidResolution[];
  totalSize: number;
  checkedCids: Set<string>;
}

const emptyResolveResults: ResolveResults = {
  resolutions: [],
  totalSize: 0,
  checkedCids: new Set(),
};

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
  const [activeTab, setActiveTab] = useState<string>("by-cid");

  // Form inputs (block+index tab)
  const [blockInput, setBlockInput] = useState("");
  const [indexInput, setIndexInput] = useState("");

  // Lookup state (block+index tab)
  const [isLookingUp, setIsLookingUp] = useState(false);
  const [lookupError, setLookupError] = useState<string | null>(null);
  const [renewalTarget, setRenewalTarget] = useState<RenewalTarget | null>(null);

  // Renewal state (block+index tab)
  const [isRenewing, setIsRenewing] = useState(false);
  /** Which action is in flight, so only its button shows the spinner. */
  const [renewingAction, setRenewingAction] = useState<RenewAction>("scheduled");
  const [renewalError, setRenewalError] = useState<string | null>(null);
  const [renewalSuccess, setRenewalSuccess] = useState<{
    action: RenewAction;
    blockNumber?: number;
    /** Set only for `immediate`; the registering actions do not move the expiry yet. */
    newExpiresAt?: number;
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
  const [resolveResults, setResolveResults] = useState<ResolveResults>(emptyResolveResults);
  const { resolutions, totalSize, checkedCids } = resolveResults;

  // Batch renewal state
  const [isBatchRenewing, setIsBatchRenewing] = useState(false);
  const [batchError, setBatchError] = useState<string | null>(null);
  const [batchResults, setBatchResults] = useState<BatchRenewResult[]>([]);
  const [batchAction, setBatchAction] = useState<RenewAction>("scheduled");
  /** Content-hash hex of the registration currently being disabled. */
  const [isDisabling, setIsDisabling] = useState<string | null>(null);
  const [disableError, setDisableError] = useState<string | null>(null);
  const [batchProgress, setBatchProgress] = useState<string | null>(null);
  const [copiedCid, setCopiedCid] = useState<string | null>(null);

  const handleCopyCid = useCallback(async (cid: string) => {
    try {
      await navigator.clipboard.writeText(cid);
      setCopiedCid(cid);
      setTimeout(() => setCopiedCid((c) => (c === cid ? null : c)), 1500);
    } catch {
      // Clipboard API blocked — silently ignore.
    }
  }, []);

  // Renewal registrations by content-hash hex. The pallet allows one per hash,
  // so an existing entry blocks both `renew` and `enable_auto_renew`.
  const [registrations, setRegistrations] = useState<
    Map<string, RenewalRegistration | null>
  >(new Map());

  const refreshRegistrations = useCallback(
    async (client: NonNullable<typeof api>, hashes: string[]) => {
      if (hashes.length === 0) return;
      const entries = await Promise.all(
        hashes.map(
          async (h) =>
            [h, await fetchRenewalRegistration(client, h as HexString)] as const,
        ),
      );
      setRegistrations((prev) => {
        const next = new Map(prev);
        for (const [h, reg] of entries) next.set(h, reg);
        return next;
      });
    },
    [],
  );

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
      await refreshRegistrations(api, [bytesToHex(info.contentHash)]);
    } catch (err) {
      console.error("Lookup failed:", err);
      setLookupError(err instanceof Error ? err.message : "Failed to lookup transaction");
    } finally {
      setIsLookingUp(false);
    }
  }, [api, blockInput, indexInput, retentionPeriod, refreshRegistrations]);

  const handleRenew = useCallback(async (action: RenewAction) => {
    if (!api || !selectedAccount?.polkadotSigner || !renewalTarget) return;

    setIsRenewing(true);
    setRenewingAction(action);
    setRenewalError(null);
    setRenewalSuccess(null);
    setTxStatus(null);

    const contentHashHex = bytesToHex(renewalTarget.info.contentHash);

    try {
      // Create SDK client with user's signer
      const bulletinClient = createBulletinClient!(selectedAccount.polkadotSigner);

      const result = await submitRenewAction(
        bulletinClient,
        action,
        {
          block: renewalTarget.blockNumber,
          index: renewalTarget.index,
          contentHash: renewalTarget.info.contentHash,
        },
        WaitFor.Finalized,
        handleProgress,
      );

      // Only an immediate renewal moves the expiry; the registering actions
      // fire at the retention boundary, so there is no new expiry to report.
      // retentionPeriod is guaranteed non-null here.
      const renewedAtBlock = result.blockNumber ?? (currentBlockNumber ?? 0);
      setRenewalSuccess({
        action,
        blockNumber: result.blockNumber,
        newExpiresAt:
          action === "immediate" ? renewedAtBlock + retentionPeriod! : undefined,
      });

      // Clear the target after an immediate renewal; keep it for the
      // registering actions so the new registration badge is visible.
      if (action === "immediate") {
        setRenewalTarget(null);
      }
      await refreshRegistrations(api, [contentHashHex]);
    } catch (err) {
      console.error("Renewal failed:", err);
      setRenewalError(renewErrorMessage(err, action));
    } finally {
      setIsRenewing(false);
      setTxStatus(null);
    }
  }, [
    api,
    selectedAccount,
    renewalTarget,
    currentBlockNumber,
    retentionPeriod,
    createBulletinClient,
    handleProgress,
    refreshRegistrations,
  ]);

  const handleDisableAutoRenew = useCallback(
    async (contentHash: Uint8Array, hashHex: string) => {
      if (!api || !selectedAccount?.polkadotSigner) return;
      setIsDisabling(hashHex);
      setDisableError(null);
      try {
        const bulletinClient = createBulletinClient!(selectedAccount.polkadotSigner);
        await bulletinClient
          .disableAutoRenew(contentHash)
          .withCallback(handleProgress)
          .withWaitFor(WaitFor.Finalized)
          .send();
        await refreshRegistrations(api, [hashHex]);
      } catch (err) {
        console.error("Disable auto-renew failed:", err);
        setDisableError(renewErrorMessage(err, "auto"));
      } finally {
        setIsDisabling(null);
        setTxStatus(null);
      }
    },
    [api, selectedAccount, createBulletinClient, handleProgress, refreshRegistrations],
  );

  // CID input handler
  const handleCidChange = (value: string, isValid: boolean, cid?: CID) => {
    setCidInput(value);
    setIsCidValid(isValid);
    setParsedCid(cid);
    // Clear previous resolution when CID changes
    setResolveResults(emptyResolveResults);
    setResolveError(null);
    setBatchError(null);
    setBatchResults([]);
  };

  // Resolve CID to on-chain locations
  const handleResolveCid = useCallback(async () => {
    if (!api || !parsedCid) return;

    setIsResolving(true);
    setResolveError(null);
    setResolveProgress(null);
    setResolveResults(emptyResolveResults);
    setBatchError(null);
    setBatchResults([]);

    const gatewayUrl = currentNetwork.ipfsGateway;
    if (isDagPb(parsedCid) && !gatewayUrl) {
      setResolveError(
        `No IPFS gateway configured for the "${currentNetwork.id}" network. ` +
        `Use the "By Block + Index" tab to renew manually.`,
      );
      setIsResolving(false);
      return;
    }

    // Build local hints from the current network's upload history.
    const localHints = new Map<string, OnChainTransaction>(
      networkHistory.map((e) => [
        e.contentHash,
        { blockNumber: e.blockNumber, index: e.index },
      ]),
    );

    try {
      const { resolutions: resolved, totalSize: parsedTotalSize } = await resolveCid(
        api,
        parsedCid,
        (cidStr) => fetchRawBlock(cidStr, gatewayUrl ?? ""),
        {
          localHints,
          onProgress: (phase) => {
            setResolveProgress(
              phase === "fetch-manifest"
                ? "Fetching DAG-PB manifest..."
                : "Looking up on-chain transactions...",
            );
          },
        },
      );

      // Check all found CIDs by default
      setResolveResults({
        resolutions: resolved,
        totalSize: parsedTotalSize,
        checkedCids: new Set(
          resolved.filter((r) => r.location !== null).map((r) => r.cidString),
        ),
      });

      if (resolved.every((r) => r.location === null)) {
        setResolveError(
          "None of the CIDs were found on chain. The data may have expired or was never stored on this network.",
        );
      }

      await refreshRegistrations(
        api,
        resolved.filter((r) => r.location !== null).map((r) => r.contentHashHex),
      );
    } catch (err) {
      console.error("CID resolution failed:", err);
      setResolveError(err instanceof Error ? err.message : "Failed to resolve CID");
    } finally {
      setIsResolving(false);
      setResolveProgress(null);
    }
  }, [api, parsedCid, currentNetwork, networkHistory, refreshRegistrations]);

  // Toggle a single CID checkbox
  const handleToggleCid = (cidString: string) => {
    setResolveResults((prev) => {
      const next = new Set(prev.checkedCids);
      if (next.has(cidString)) {
        next.delete(cidString);
      } else {
        next.add(cidString);
      }
      return { ...prev, checkedCids: next };
    });
  };

  const handleSelectAll = () => {
    setResolveResults((prev) => ({
      ...prev,
      checkedCids: new Set(
        prev.resolutions.filter((r) => r.location !== null).map((r) => r.cidString),
      ),
    }));
  };

  const handleDeselectAll = () => {
    setResolveResults((prev) => ({ ...prev, checkedCids: new Set() }));
  };

  // Renew each selected CID with its own signed extrinsic. The chain's
  // ValidateStorageCalls extension rejects store/renew wrapped in Utility
  // batches (`pallets/transaction-storage/src/extension.rs:244`), so batching
  // isn't an option — sequential per-CID renewals are required.
  const handleBatchRenew = useCallback(async (action: RenewAction) => {
    if (!api || !selectedAccount?.polkadotSigner) return;

    const targets = resolutions.filter(
      (r) => r.location !== null && checkedCids.has(r.cidString),
    );
    if (targets.length === 0) return;

    setIsBatchRenewing(true);
    setBatchAction(action);
    setBatchError(null);
    setBatchResults([]);
    setBatchProgress(null);

    const bulletinClient = createBulletinClient!(selectedAccount.polkadotSigner);
    const results: BatchRenewResult[] = [];
    const verb = action === "auto" ? "Enabling auto-renew for" : "Renewing";

    for (let i = 0; i < targets.length; i++) {
      const t = targets[i]!;
      const cidStr = t.cidString;
      const shortCid =
        cidStr.length > 20 ? `${cidStr.slice(0, 10)}...${cidStr.slice(-6)}` : cidStr;
      setBatchProgress(`${verb} ${i + 1} of ${targets.length}: ${shortCid}`);

      try {
        // Per-tx InBlock (not Finalized): these are safe to retry — a
        // reorg-dropped one can simply be resubmitted. Saves ~6s per CID.
        const result = await submitRenewAction(
          bulletinClient,
          action,
          {
            block: t.location!.blockNumber,
            index: t.location!.index,
            contentHash: t.cid.multihash.digest,
          },
          WaitFor.InBlock,
          handleProgress,
        );

        // Only an immediate renewal moves the expiry.
        const renewedAtBlock = result.blockNumber ?? currentBlockNumber ?? 0;
        results.push({
          cidString: cidStr,
          success: true,
          newExpiresAt:
            action === "immediate"
              ? renewedAtBlock + (retentionPeriod ?? 0)
              : undefined,
        });
      } catch (err) {
        console.error(`Failed to renew ${cidStr}:`, err);
        results.push({
          cidString: cidStr,
          success: false,
          error: renewErrorMessage(err, action),
        });
      }
    }

    setBatchResults(results);
    setIsBatchRenewing(false);
    setBatchProgress(null);
    setTxStatus(null);
    await refreshRegistrations(
      api,
      targets.map((t) => t.contentHashHex),
    );
  }, [
    api,
    selectedAccount,
    resolutions,
    checkedCids,
    currentBlockNumber,
    retentionPeriod,
    createBulletinClient,
    handleProgress,
    refreshRegistrations,
  ]);

  const canRenew =
    api &&
    selectedAccount?.polkadotSigner &&
    renewalTarget &&
    !isRenewing;

  // A content hash carries at most one registration, so `renew` and
  // `enable_auto_renew` both reject once one exists.
  const targetRegistration = renewalTarget
    ? registrations.get(bytesToHex(renewalTarget.info.contentHash)) ?? null
    : null;

  // Calculate blocks until expiration
  const blocksUntilExpiration = renewalTarget && currentBlockNumber !== undefined
    ? renewalTarget.expiresAtBlock - currentBlockNumber
    : null;

  const isExpired = blocksUntilExpiration !== null && blocksUntilExpiration <= 0;
  const isExpiringSoon =
    blocksUntilExpiration !== null &&
    blocksUntilExpiration > 0 &&
    blocksUntilExpiration < EXPIRING_SOON_BLOCKS;

  // CID tab helpers
  const checkedCount = resolutions.filter((r) => r.location !== null && checkedCids.has(r.cidString)).length;
  const foundCount = resolutions.filter((r) => r.location !== null).length;
  const canBatchRenew = api && selectedAccount?.polkadotSigner && checkedCount > 0 && !isBatchRenewing;

  // Calculate expiration for a resolution
  const getExpirationInfo = (resolution: CidResolution) => {
    if (resolution.location === null || retentionPeriod === null || currentBlockNumber === undefined) {
      return null;
    }
    const expiresAt = resolution.location.blockNumber + retentionPeriod;
    const remaining = expiresAt - currentBlockNumber;
    return {
      expiresAt,
      remaining,
      expired: remaining <= 0,
      expiringSoon: remaining > 0 && remaining < EXPIRING_SOON_BLOCKS,
    };
  };

  // Selected CIDs that block a registering action, and the soonest retention
  // deadline among the selection — the number that matters for a scheduled renew.
  const selectedResolutions = resolutions.filter(
    (r) => r.location !== null && checkedCids.has(r.cidString),
  );
  const selectedRegistered = selectedResolutions.filter((r) =>
    registrations.get(r.contentHashHex),
  ).length;
  const selectedExpired = selectedResolutions.filter(
    (r) => getExpirationInfo(r)?.expired,
  ).length;
  const selectedMinRemaining = selectedResolutions.length
    ? Math.min(
        ...selectedResolutions.map((r) => getExpirationInfo(r)?.remaining ?? 0),
      )
    : null;

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
                  <TabsTrigger value="by-cid">By CID</TabsTrigger>
                  <TabsTrigger value="block-index">By Block + Index</TabsTrigger>
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
                      <div className="flex items-center justify-between flex-wrap gap-2">
                        <div className="text-sm text-muted-foreground">
                          Found {foundCount} of {resolutions.length} CID(s) on chain
                          {totalSize > 0 && (
                            <> · Total size: {formatBytes(totalSize)}</>
                          )}
                        </div>
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
                          const registration = registrations.get(r.contentHashHex);
                          const shortCid = r.cidString.length > 30
                            ? `${r.cidString.slice(0, 14)}...${r.cidString.slice(-8)}`
                            : r.cidString;
                          const found = r.location !== null;

                          return (
                            <div
                              key={r.cidString}
                              className="flex items-center gap-3 p-3 rounded-md border bg-card"
                            >
                              <input
                                type="checkbox"
                                checked={checkedCids.has(r.cidString)}
                                disabled={!found}
                                onChange={() => handleToggleCid(r.cidString)}
                                className="h-4 w-4 rounded border-input accent-primary"
                              />
                              <div className="flex-1 min-w-0 space-y-1">
                                <div className="flex items-center gap-2 flex-wrap">
                                  <span className="font-mono text-xs truncate" title={r.cidString}>
                                    {shortCid}
                                  </span>
                                  <button
                                    type="button"
                                    onClick={() => handleCopyCid(r.cidString)}
                                    className="text-muted-foreground hover:text-foreground transition-colors"
                                    title="Copy CID"
                                    aria-label="Copy CID"
                                  >
                                    {copiedCid === r.cidString ? (
                                      <Check className="h-3.5 w-3.5 text-green-600" />
                                    ) : (
                                      <Copy className="h-3.5 w-3.5" />
                                    )}
                                  </button>
                                  <Badge variant="secondary" className="text-xs">
                                    {r.isManifest ? "Manifest" : `Chunk ${i}`}
                                  </Badge>
                                </div>
                                <div className="flex items-center gap-2 flex-wrap">
                                  {r.location ? (
                                    <>
                                      <span className="text-xs text-muted-foreground">
                                        Block #{r.location.blockNumber}, Index #{r.location.index}
                                      </span>
                                      {expInfo && (
                                        <>
                                          {expInfo.expired ? (
                                            <Badge variant="destructive" className="text-xs">Expired</Badge>
                                          ) : expInfo.expiringSoon ? (
                                            <Badge className="bg-amber-500/10 text-amber-600 border-amber-500/20 text-xs">
                                              {formatBlockDuration(expInfo.remaining)} left
                                            </Badge>
                                          ) : (
                                            <Badge variant="secondary" className="bg-green-500/10 text-green-600 text-xs">
                                              {formatBlockDuration(expInfo.remaining)} left
                                            </Badge>
                                          )}
                                        </>
                                      )}
                                      {registration && (
                                        <RegistrationBadge
                                          registration={registration}
                                          busy={isDisabling === r.contentHashHex}
                                          onDisable={() =>
                                            handleDisableAutoRenew(
                                              r.cid.multihash.digest,
                                              r.contentHashHex,
                                            )
                                          }
                                        />
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

                      {/* Renewal actions */}
                      {selectedMinRemaining !== null && (
                        <p className="text-sm">
                          <span className="text-muted-foreground">Retention </span>
                          <span className="font-medium">
                            {selectedMinRemaining <= 0
                              ? "expired"
                              : `${formatBlockDuration(selectedMinRemaining)} left`}
                          </span>
                          {checkedCount > 1 && (
                            <span className="text-muted-foreground">
                              {" "}(earliest of {checkedCount} selected)
                            </span>
                          )}
                        </p>
                      )}

                      <div className="grid gap-2 sm:grid-cols-3">
                        <Button
                          onClick={() => handleBatchRenew("scheduled")}
                          disabled={!canBatchRenew || selectedRegistered > 0 || selectedExpired > 0}
                        >
                          {isBatchRenewing && batchAction === "scheduled" ? (
                            <Spinner size="sm" className="mr-2" />
                          ) : (
                            <Clock className="h-4 w-4 mr-2" />
                          )}
                          Renew selected ({checkedCount})
                        </Button>
                        <Button
                          variant="outline"
                          onClick={() => handleBatchRenew("immediate")}
                          disabled={!canBatchRenew}
                        >
                          {isBatchRenewing && batchAction === "immediate" ? (
                            <Spinner size="sm" className="mr-2" />
                          ) : (
                            <RefreshCw className="h-4 w-4 mr-2" />
                          )}
                          Renew immediately
                        </Button>
                        <Button
                          variant="outline"
                          onClick={() => handleBatchRenew("auto")}
                          disabled={!canBatchRenew || selectedRegistered > 0}
                        >
                          {isBatchRenewing && batchAction === "auto" ? (
                            <Spinner size="sm" className="mr-2" />
                          ) : (
                            <RefreshCw className="h-4 w-4 mr-2" />
                          )}
                          Enable auto-renew
                        </Button>
                      </div>

                      <RenewActionLegend />

                      {selectedRegistered > 0 && (
                        <p className="text-xs text-amber-600">
                          {selectedRegistered} selected CID(s) already have a renewal
                          registered.
                        </p>
                      )}
                      {selectedExpired > 0 && (
                        <p className="text-xs text-amber-600">
                          Expired entries can only be renewed immediately.
                        </p>
                      )}

                      {!selectedAccount && (
                        <p className="text-sm text-muted-foreground text-center">
                          Connect a wallet to renew data
                        </p>
                      )}
                    </div>
                  )}

                  {/* Per-CID progress while renewing */}
                  {isBatchRenewing && batchProgress && (
                    <div className="flex items-start gap-3 p-3 rounded-md bg-secondary/50 text-sm">
                      <Spinner size="sm" className="mt-0.5 shrink-0" />
                      <div className="space-y-0.5 min-w-0">
                        <p className="font-medium">{batchProgress}</p>
                        {txStatus && (
                          <p className="text-xs text-muted-foreground">{txStatus}</p>
                        )}
                      </div>
                    </div>
                  )}

                  {/* Per-CID renewal results */}
                  {batchResults.length > 0 && (() => {
                    const succeeded = batchResults.filter((r) => r.success).length;
                    const failed = batchResults.length - succeeded;
                    return (
                      <div className="space-y-2">
                        <div
                          className={cn(
                            "flex items-start gap-3 p-3 rounded-md",
                            failed === 0
                              ? "bg-green-500/10 text-green-700 dark:text-green-400"
                              : "bg-yellow-500/10 text-yellow-700 dark:text-yellow-400",
                          )}
                        >
                          {failed === 0 ? (
                            <Check className="h-5 w-5 mt-0.5 shrink-0" />
                          ) : (
                            <AlertCircle className="h-5 w-5 mt-0.5 shrink-0" />
                          )}
                          <div className="text-sm">
                            <p className="font-medium">
                              {BATCH_SUMMARY_VERBS[batchAction]} {succeeded} of{" "}
                              {batchResults.length} CID(s)
                              {failed > 0 && ` — ${failed} failed`}
                            </p>
                          </div>
                        </div>
                        {failed > 0 && (
                          <div className="space-y-1">
                            {batchResults
                              .filter((r) => !r.success)
                              .map((r) => {
                                const shortCid =
                                  r.cidString.length > 20
                                    ? `${r.cidString.slice(0, 10)}...${r.cidString.slice(-6)}`
                                    : r.cidString;
                                return (
                                  <div
                                    key={r.cidString}
                                    className="text-xs text-destructive flex items-center gap-1.5"
                                  >
                                    <span className="font-mono">{shortCid}</span>
                                    <button
                                      type="button"
                                      onClick={() => handleCopyCid(r.cidString)}
                                      className="text-destructive/70 hover:text-destructive transition-colors"
                                      title="Copy CID"
                                      aria-label="Copy CID"
                                    >
                                      {copiedCid === r.cidString ? (
                                        <Check className="h-3 w-3" />
                                      ) : (
                                        <Copy className="h-3 w-3" />
                                      )}
                                    </button>
                                    <span>: {r.error}</span>
                                  </div>
                                );
                              })}
                          </div>
                        )}
                      </div>
                    );
                  })()}

                  {batchError && (
                    <div className="flex items-start gap-3 p-3 rounded-md bg-destructive/10 text-destructive">
                      <AlertCircle className="h-5 w-5 mt-0.5 shrink-0" />
                      <div className="text-sm">
                        <p className="font-medium">Batch renewal failed</p>
                        <p className="mt-1">{batchError}</p>
                      </div>
                    </div>
                  )}

                  {disableError && (
                    <div className="flex items-start gap-3 p-3 rounded-md bg-destructive/10 text-destructive">
                      <AlertCircle className="h-5 w-5 mt-0.5 shrink-0" />
                      <div className="text-sm">
                        <p className="font-medium">Disable failed</p>
                        <p className="mt-1">{disableError}</p>
                      </div>
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
                  <div className="flex items-center gap-2 flex-wrap">
                    {isExpired ? (
                      <Badge variant="destructive">Expired</Badge>
                    ) : isExpiringSoon ? (
                      <Badge className="bg-amber-500/10 text-amber-600 border-amber-500/20">
                        {formatBlockDuration(blocksUntilExpiration!)} left
                      </Badge>
                    ) : (
                      <Badge variant="secondary" className="bg-green-500/10 text-green-600">
                        {blocksUntilExpiration !== null
                          ? `${formatBlockDuration(blocksUntilExpiration)} left`
                          : "Active"}
                      </Badge>
                    )}
                    {targetRegistration && (
                      <RegistrationBadge
                        registration={targetRegistration}
                        busy={isDisabling !== null}
                        onDisable={() =>
                          handleDisableAutoRenew(
                            renewalTarget.info.contentHash,
                            bytesToHex(renewalTarget.info.contentHash),
                          )
                        }
                      />
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

                <div className="grid gap-2 sm:grid-cols-3">
                  <Button
                    onClick={() => handleRenew("scheduled")}
                    disabled={!canRenew || !!targetRegistration || isExpired}
                  >
                    {isRenewing && renewingAction === "scheduled" ? (
                      <Spinner size="sm" className="mr-2" />
                    ) : (
                      <Clock className="h-4 w-4 mr-2" />
                    )}
                    Renew
                  </Button>
                  <Button
                    variant="outline"
                    onClick={() => handleRenew("immediate")}
                    disabled={!canRenew}
                  >
                    {isRenewing && renewingAction === "immediate" ? (
                      <Spinner size="sm" className="mr-2" />
                    ) : (
                      <RefreshCw className="h-4 w-4 mr-2" />
                    )}
                    Renew immediately
                  </Button>
                  <Button
                    variant="outline"
                    onClick={() => handleRenew("auto")}
                    disabled={!canRenew || !!targetRegistration}
                  >
                    {isRenewing && renewingAction === "auto" ? (
                      <Spinner size="sm" className="mr-2" />
                    ) : (
                      <RefreshCw className="h-4 w-4 mr-2" />
                    )}
                    Enable auto-renew
                  </Button>
                </div>

                {isRenewing && txStatus && (
                  <p className="text-xs text-muted-foreground text-center">{txStatus}</p>
                )}

                <RenewActionLegend />

                {targetRegistration && (
                  <p className="text-xs text-amber-600">
                    A renewal is already registered for this content hash.
                  </p>
                )}
                {isExpired && (
                  <p className="text-xs text-amber-600">
                    Expired entries can only be renewed immediately.
                  </p>
                )}

                {(renewalError || disableError) && (
                  <div className="flex items-start gap-3 p-3 rounded-md bg-destructive/10 text-destructive">
                    <AlertCircle className="h-5 w-5 mt-0.5" />
                    <div>
                      <p className="font-medium">
                        {renewalError ? "Renewal Failed" : "Disable Failed"}
                      </p>
                      <p className="text-sm mt-1">{renewalError ?? disableError}</p>
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
                  {SUCCESS_TITLES[renewalSuccess.action]}
                </CardTitle>
                <CardDescription>
                  {SUCCESS_DESCRIPTIONS[renewalSuccess.action]}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="grid sm:grid-cols-2 gap-4">
                  {renewalSuccess.blockNumber && (
                    <div className="space-y-1">
                      <p className="text-xs text-muted-foreground uppercase tracking-wide">
                        Submitted in Block
                      </p>
                      <p className="font-mono">
                        #{renewalSuccess.blockNumber.toLocaleString()}
                      </p>
                    </div>
                  )}
                  {renewalSuccess.newExpiresAt !== undefined && (
                    <div className="space-y-1">
                      <p className="text-xs text-muted-foreground uppercase tracking-wide">
                        New Expiration Block
                      </p>
                      <p className="font-mono">
                        #{renewalSuccess.newExpiresAt.toLocaleString()}
                      </p>
                    </div>
                  )}
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
              <p>
                On networks whose runtime predates immediate renewal, a plain renewal
                already takes effect at once — there <strong>Renew</strong> and{" "}
                <strong>Renew immediately</strong> behave identically, and auto-renew is
                unavailable.
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
          <CidInfoCard cid={parsedCid} />

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
