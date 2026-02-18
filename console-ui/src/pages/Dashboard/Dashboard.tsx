import { useState, useEffect } from "react";
import { Link } from "react-router-dom";
import { Upload, Download, Search, Shield, Database, Activity, BarChart3 } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import { AuthorizationCard } from "@/components/AuthorizationCard";
import { useChainState, useApi, StorageType } from "@/state/chain.state";
import { useSelectedAccount } from "@/state/wallet.state";
import { formatAddress, formatBlockNumber, formatBytes, formatNumber } from "@/utils/format";

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
          <Link to="/authorizations">
            <Button variant="outline" className="w-full justify-start">
              <Shield className="h-4 w-4 mr-2" />
              View Authorizations
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
          <div className="flex items-center justify-between">
            <span className="text-sm text-muted-foreground">Network</span>
            <Badge variant="secondary">{network.name}</Badge>
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

function AccountCard() {
  const selectedAccount = useSelectedAccount();

  if (!selectedAccount) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Account</CardTitle>
          <CardDescription>Connect a wallet to get started</CardDescription>
        </CardHeader>
        <CardContent>
          <Link to="/accounts">
            <Button className="w-full">Connect Wallet</Button>
          </Link>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Connected Account</CardTitle>
        <CardDescription>{selectedAccount.name || "Unknown"}</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-2">
          <p className="font-mono text-sm break-all">
            {formatAddress(selectedAccount.address, 8)}
          </p>
          <Link to="/accounts">
            <Button variant="outline" size="sm" className="w-full">
              Manage Accounts
            </Button>
          </Link>
        </div>
      </CardContent>
    </Card>
  );
}

function WelcomeCard({ storageType }: { storageType: StorageType }) {
  if (storageType === "web3storage") {
    return (
      <Card className="col-span-full bg-gradient-to-br from-primary/10 to-accent/10 border-primary/20">
        <CardHeader>
          <CardTitle className="text-2xl">Welcome to Web3 Storage Console</CardTitle>
          <CardDescription className="text-base">
            Decentralized storage powered by Web3 infrastructure
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="grid sm:grid-cols-3 gap-4 text-sm">
            <div className="space-y-1">
              <p className="font-medium">Web3 Native</p>
              <p className="text-muted-foreground">
                Built for the decentralized web
              </p>
            </div>
            <div className="space-y-1">
              <p className="font-medium">Content Addressed</p>
              <p className="text-muted-foreground">
                Data identified and verified by content hashes
              </p>
            </div>
            <div className="space-y-1">
              <p className="font-medium">Permissionless</p>
              <p className="text-muted-foreground">
                Open access to store and retrieve data
              </p>
            </div>
          </div>
        </CardContent>
      </Card>
    );
  }

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
  userAuths: { count: number; bytes: bigint };
  preimageAuths: { count: number; bytes: bigint };
  transactions: { count: number; bytes: bigint };
}

function UsageCard() {
  const api = useApi();
  const [stats, setStats] = useState<UsageStats | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!api) return;

    let cancelled = false;
    setLoading(true);

    Promise.all([
      api.query.TransactionStorage.Authorizations.getEntries(),
      api.query.TransactionStorage.Transactions.getEntries(),
    ])
      .then(([authEntries, txEntries]: [any[], any[]]) => {
        if (cancelled) return;

        const userAuths = { count: 0, bytes: 0n };
        const preimageAuths = { count: 0, bytes: 0n };

        for (const { keyArgs, value } of authEntries) {
          const extent = value.extent;
          if (keyArgs[0].type === "Account") {
            userAuths.count += Number(extent.transactions);
            userAuths.bytes += BigInt(extent.bytes);
          } else if (keyArgs[0].type === "Preimage") {
            preimageAuths.count += Number(extent.transactions);
            preimageAuths.bytes += BigInt(extent.bytes);
          }
        }

        let txCount = 0;
        let txBytes = 0n;
        for (const { value } of txEntries) {
          if (Array.isArray(value)) {
            for (const info of value) {
              txCount++;
              txBytes += BigInt(info.size);
            }
          }
        }

        setStats({
          userAuths,
          preimageAuths,
          transactions: { count: txCount, bytes: txBytes },
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
        ) : (
          <div className="space-y-4">
            <div>
              <p className="text-sm font-medium mb-2">Transactions</p>
              <p className="text-sm pl-2 text-right">
                {formatNumber(stats.transactions.count)} txns &middot; {formatBytes(stats.transactions.bytes)}
              </p>
            </div>
            <div>
              <p className="text-sm font-medium mb-2">Authorizations</p>
              <div className="space-y-1 pl-2">
                <div className="flex items-center justify-between">
                  <span className="text-xs text-muted-foreground">User</span>
                  <span className="text-sm">
                    {formatNumber(stats.userAuths.count)} txns &middot; {formatBytes(stats.userAuths.bytes)}
                  </span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-xs text-muted-foreground">Preimage</span>
                  <span className="text-sm">
                    {formatNumber(stats.preimageAuths.count)} txns &middot; {formatBytes(stats.preimageAuths.bytes)}
                  </span>
                </div>
              </div>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function Dashboard() {
  const { storageType } = useChainState();

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Dashboard</h1>
        <p className="text-muted-foreground">
          {storageType === "web3storage"
            ? "Overview of your Web3 Storage activity"
            : "Overview of your Bulletin Chain activity"}
        </p>
      </div>

      {storageType === "web3storage" ? (
        <div className="grid gap-6 md:grid-cols-2">
          <WelcomeCard storageType={storageType} />
          <ChainInfoCard />
        </div>
      ) : (
        <div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
          <WelcomeCard storageType={storageType} />
          <ChainInfoCard />
          <QuickActions />
          <AccountCard />
          <UsageCard />
          <AuthorizationCard className="lg:col-start-3" />
        </div>
      )}
    </div>
  );
}
