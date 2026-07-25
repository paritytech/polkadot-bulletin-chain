// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

import { useState, useEffect } from "react";
import { Link } from "react-router-dom";
import { Upload, Download, Search, Shield, Database, Activity, BarChart3, Droplets, RefreshCw } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import { AccountSummaryCard } from "@/components/AccountSummaryCard";
import { PalletUnavailableNotice } from "@/components/PalletUnavailableNotice";
import { useChainState, useApi } from "@/state/chain.state";
import {
  extentAllowanceBytes,
  type RawTransactionInfo,
  entryKindOf,
} from "@/state/storage.state";
import { formatBlockNumber, formatBytes, formatNumber } from "@/utils/format";

function QuickActions() {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Activity className="h-5 w-5" />
          Quick Actions
        </CardTitle>
        <CardDescription>Common operations</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="grid grid-cols-2 gap-3">
          <Link to="/upload">
            <Button variant="outline" className="w-full justify-start">
              <Upload className="h-4 w-4 mr-2" />
              Upload Data
            </Button>
          </Link>
          <Link to="/download">
            <Button variant="outline" className="w-full justify-start">
              <Download className="h-4 w-4 mr-2" />
              Download by CID
            </Button>
          </Link>
          <Link to="/explorer">
            <Button variant="outline" className="w-full justify-start">
              <Search className="h-4 w-4 mr-2" />
              Explore Blocks
            </Button>
          </Link>
          <Link to="/renew">
            <Button variant="outline" className="w-full justify-start">
              <RefreshCw className="h-4 w-4 mr-2" />
              Renew Storage
            </Button>
          </Link>
          <Link to="/authorizations">
            <Button variant="outline" className="w-full justify-start">
              <Shield className="h-4 w-4 mr-2" />
              View Authorizations
            </Button>
          </Link>
          <Link to="/authorizations?tab=faucet">
            <Button variant="outline" className="w-full justify-start">
              <Droplets className="h-4 w-4 mr-2" />
              Storage Faucet
            </Button>
          </Link>
        </div>
      </CardContent>
    </Card>
  );
}

function ChainInfoCard() {
  const { status, chainName, specVersion, tokenSymbol, blockNumber, network } = useChainState();

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Database className="h-5 w-5" />
          Chain Info
        </CardTitle>
        <CardDescription>Current network status</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-3">
          <div>
            <div className="flex items-center justify-between">
              <span className="text-sm text-muted-foreground">Network</span>
              <Badge variant="secondary">{network.name}</Badge>
            </div>
            {network.endpoints[0] && (
              <p className="text-xs text-muted-foreground font-mono mt-1 text-right break-all">
                {network.endpoints[0]}
              </p>
            )}
          </div>
          <div className="flex items-center justify-between">
            <span className="text-sm text-muted-foreground">Status</span>
            <Badge
              variant={
                status === "connected"
                  ? "success"
                  : status === "connecting"
                  ? "warning"
                  : "secondary"
              }
            >
              {status}
            </Badge>
          </div>
          {chainName && (
            <div className="flex items-center justify-between">
              <span className="text-sm text-muted-foreground">Runtime</span>
              <span className="text-sm font-mono">{chainName}</span>
            </div>
          )}
          {specVersion !== undefined && (
            <div className="flex items-center justify-between">
              <span className="text-sm text-muted-foreground">Spec Version</span>
              <span className="text-sm font-mono">{specVersion}</span>
            </div>
          )}
          {blockNumber !== undefined && (
            <div className="flex items-center justify-between">
              <span className="text-sm text-muted-foreground">Block</span>
              <span className="text-sm font-mono">{formatBlockNumber(blockNumber)}</span>
            </div>
          )}
          {tokenSymbol && (
            <div className="flex items-center justify-between">
              <span className="text-sm text-muted-foreground">Token</span>
              <span className="text-sm">{tokenSymbol}</span>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function WelcomeCard() {
  return (
    <Card className="col-span-full bg-gradient-to-br from-primary/10 to-accent/10 border-primary/20">
      <CardHeader>
        <CardTitle className="text-2xl">Welcome to Bulletin Chain Console</CardTitle>
        <CardDescription className="text-base">
          Store and retrieve data on the Polkadot Bulletin Chain with IPFS integration
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="grid sm:grid-cols-3 gap-4 text-sm">
          <div className="space-y-1">
            <p className="font-medium">Decentralized Storage</p>
            <p className="text-muted-foreground">
              Store data with proof-of-storage guarantees
            </p>
          </div>
          <div className="space-y-1">
            <p className="font-medium">IPFS Compatible</p>
            <p className="text-muted-foreground">
              Access stored data via standard IPFS CIDs
            </p>
          </div>
          <div className="space-y-1">
            <p className="font-medium">Authorization Based</p>
            <p className="text-muted-foreground">
              Manage storage quotas and permissions
            </p>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

interface UsageStats {
  ephemeral: { count: number; bytes: bigint };
  permanent: { count: number; used: bigint; cap: bigint };
  userAuths: { count: number; bytes: bigint };
  preimageAuths: { count: number; bytes: bigint };
}

function UsageCard() {
  const api = useApi();
  const [stats, setStats] = useState<UsageStats | null>(null);
  const [loading, setLoading] = useState(false);
  const [palletError, setPalletError] = useState<string | null>(null);
  const [retentionPeriod, setRetentionPeriod] = useState<number | null>(null);

  useEffect(() => {
    if (!api) return;
    let cancelled = false;
    api.query.TransactionStorage.RetentionPeriod.getValue()
      .then((p: bigint | number) => {
        if (!cancelled) setRetentionPeriod(Number(p));
      })
      .catch(() => {
        /* surfaced via palletError from the main effect */
      });
    return () => {
      cancelled = true;
    };
  }, [api]);

  useEffect(() => {
    if (!api) return;

    let cancelled = false;
    setLoading(true);
    setPalletError(null);

    const recordPalletError = (err: unknown) => {
      const msg = err instanceof Error ? err.message : String(err);
      if (!cancelled) setPalletError((prev) => prev ?? msg);
    };

    Promise.all([
      api.query.TransactionStorage.Authorizations.getEntries().catch((err: unknown) => {
        recordPalletError(err);
        return null;
      }),
      api.query.TransactionStorage.Transactions.getEntries().catch((err: unknown) => {
        recordPalletError(err);
        return null;
      }),
      // The chain-wide counter and cap moved from TransactionStorage to DataRenewal;
      // fall back for chains still running the pre-split runtime.
      (
        (api.query as any).DataRenewal?.PermanentStorageUsed ??
        (api.query as any).TransactionStorage?.PermanentStorageUsed
      )
        .getValue()
        .catch((err: unknown) => {
          recordPalletError(err);
          return null;
        }),
      Promise.resolve(
        (
          (api.constants as any).DataRenewal?.MaxPermanentStorageSize ??
          (api.constants as any).TransactionStorage?.MaxPermanentStorageSize
        )()
      ).catch((err: unknown) => {
        recordPalletError(err);
        return null;
      }),
    ])
      .then(([authEntries, txEntries, permUsed, permCap]: [any[] | null, { value: RawTransactionInfo[] }[] | null, bigint | null, bigint | null]) => {
        if (cancelled) return;

        const userAuths = { count: 0, bytes: 0n };
        const preimageAuths = { count: 0, bytes: 0n };

        if (authEntries) {
          for (const { keyArgs, value } of authEntries) {
            const extent = value.extent;
            const bytesAllowance = extentAllowanceBytes(extent);
            if (keyArgs[0].type === "Account") {
              userAuths.count++;
              userAuths.bytes += bytesAllowance;
            } else if (keyArgs[0].type === "Preimage") {
              preimageAuths.count++;
              preimageAuths.bytes += bytesAllowance;
            }
          }
        }

        const ephemeral = { count: 0, bytes: 0n };
        const permanentCount = { count: 0 };
        if (txEntries) {
          for (const { value } of txEntries) {
            if (Array.isArray(value)) {
              for (const info of value) {
                const entryKind = entryKindOf(info);
                if (entryKind === "Store") {
                  ephemeral.count++;
                  ephemeral.bytes += BigInt(info.size);
                } else if (entryKind === "Renew") {
                  permanentCount.count++;
                }
              }
            }
          }
        }

        setStats({
          ephemeral,
          permanent: {
            count: permanentCount.count,
            used: permUsed ?? 0n,
            cap: permCap ?? 0n,
          },
          userAuths,
          preimageAuths,
        });
      })
      .catch((err) => {
        console.error("Failed to fetch usage stats:", err);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [api]);

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <BarChart3 className="h-5 w-5" />
          Storage Totals
        </CardTitle>
        <CardDescription>On-chain storage statistics</CardDescription>
      </CardHeader>
      <CardContent>
        {loading || !stats ? (
          <div className="flex items-center justify-center py-4">
            <Spinner size="sm" />
          </div>
        ) : palletError ? (
          <PalletUnavailableNotice pallet="TransactionStorage" details={palletError} />
        ) : (
          <div className="space-y-4">
            <div>
              <p className="text-sm font-medium mb-2">
                Ephemeral
                {retentionPeriod !== null && (
                  <span className="text-muted-foreground font-normal">
                    {" "}(RetentionPeriod: {formatNumber(retentionPeriod)})
                  </span>
                )}
              </p>
              <div className="grid grid-cols-2 gap-4">
                <TotalsStat label="Transactions" value={formatNumber(stats.ephemeral.count)} />
                <TotalsStat label="Bytes" value={formatBytes(stats.ephemeral.bytes)} />
              </div>
            </div>
            <hr />
            <div>
              <p className="text-sm font-medium mb-2">Permanent</p>
              <div className="grid grid-cols-2 gap-4">
                <TotalsStat label="Transactions" value={formatNumber(stats.permanent.count)} />
                <TotalsStat
                  label="Bytes"
                  value={formatBytes(stats.permanent.used)}
                  hint={`of ${formatBytes(stats.permanent.cap)}`}
                />
              </div>
            </div>
            <hr />
            <div>
              <p className="text-sm font-medium mb-2">Authorizations</p>
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-2">
                  <p className="text-xs text-muted-foreground">
                    Users ({formatNumber(stats.userAuths.count)})
                  </p>
                  <TotalsStat
                    label="Bytes"
                    value={formatBytes(stats.userAuths.bytes)}
                  />
                </div>
                <div className="space-y-2">
                  <p className="text-xs text-muted-foreground">
                    Preimages ({formatNumber(stats.preimageAuths.count)})
                  </p>
                  <TotalsStat
                    label="Bytes"
                    value={formatBytes(stats.preimageAuths.bytes)}
                  />
                </div>
              </div>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function TotalsStat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="space-y-1">
      <p className="text-xs text-muted-foreground uppercase tracking-wide">{label}</p>
      <p className="text-2xl font-semibold">{value}</p>
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  );
}

export function Dashboard() {
  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Dashboard</h1>
        <p className="text-muted-foreground">
          Overview of your Bulletin Chain activity
        </p>
      </div>

      <div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
        <WelcomeCard />
        <ChainInfoCard />
        <QuickActions />
        <AccountSummaryCard />
        <UsageCard />
      </div>
    </div>
  );
}
