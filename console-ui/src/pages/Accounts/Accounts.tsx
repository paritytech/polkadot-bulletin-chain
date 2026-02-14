import { useState, useEffect } from "react";
import { Wallet, Check, Copy, ExternalLink, LogOut, RefreshCw, AlertCircle } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import {
  useWalletState,
  useSelectedAccount,
  refreshExtensions,
  connectExtension,
  selectAccount,
  disconnectWallet,
} from "@/state/wallet.state";
import { formatAddress } from "@/utils/format";
import { cn } from "@/utils/cn";

const SUPPORTED_EXTENSIONS = [
  { id: "polkadot-js", name: "Polkadot.js", icon: "P" },
  { id: "subwallet-js", name: "SubWallet", icon: "S" },
  { id: "talisman", name: "Talisman", icon: "T" },
  { id: "fearless-wallet", name: "Fearless", icon: "F" },
];

function ExtensionList() {
  const { extensions, status, connectedExtension, error } = useWalletState();
  const [isConnecting, setIsConnecting] = useState<string | null>(null);

  useEffect(() => {
    refreshExtensions();
  }, []);

  const handleConnect = async (extensionId: string) => {
    setIsConnecting(extensionId);
    try {
      await connectExtension(extensionId);
    } catch (err) {
      console.error("Failed to connect:", err);
    } finally {
      setIsConnecting(null);
    }
  };

  const handleRefresh = () => {
    refreshExtensions();
  };

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle className="flex items-center gap-2">
              <Wallet className="h-5 w-5" />
              Wallet Extensions
            </CardTitle>
            <CardDescription>
              Connect a browser wallet to interact with the chain
            </CardDescription>
          </div>
          <Button variant="ghost" size="icon" onClick={handleRefresh}>
            <RefreshCw className="h-4 w-4" />
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {error && (
          <div className="mb-4 p-3 rounded-md bg-destructive/10 text-destructive text-sm flex items-center gap-2">
            <AlertCircle className="h-4 w-4" />
            {error}
          </div>
        )}

        <div className="space-y-2">
          {SUPPORTED_EXTENSIONS.map((ext) => {
            const isDetected = extensions.includes(ext.id);
            const isConnected = connectedExtension?.name === ext.id;
            const isLoading = isConnecting === ext.id;

            return (
              <div
                key={ext.id}
                className={cn(
                  "flex items-center justify-between p-3 rounded-md border transition-colors",
                  isConnected && "border-primary bg-primary/5",
                  !isDetected && "opacity-50"
                )}
              >
                <div className="flex items-center gap-3">
                  <div className="w-10 h-10 rounded-full bg-secondary flex items-center justify-center font-bold">
                    {ext.icon}
                  </div>
                  <div>
                    <p className="font-medium">{ext.name}</p>
                    <p className="text-xs text-muted-foreground">
                      {isConnected
                        ? "Connected"
                        : isDetected
                        ? "Detected"
                        : "Not installed"}
                    </p>
                  </div>
                </div>

                {isConnected ? (
                  <Badge variant="success" className="gap-1">
                    <Check className="h-3 w-3" />
                    Connected
                  </Badge>
                ) : isDetected ? (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleConnect(ext.id)}
                    disabled={isLoading || status === "connecting"}
                  >
                    {isLoading ? (
                      <>
                        <Spinner size="sm" className="mr-2" />
                        Connecting...
                      </>
                    ) : (
                      "Connect"
                    )}
                  </Button>
                ) : (
                  <Button variant="ghost" size="sm" asChild>
                    <a
                      href={`https://google.com/search?q=${ext.name}+wallet+extension`}
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      Install
                      <ExternalLink className="h-3 w-3 ml-1" />
                    </a>
                  </Button>
                )}
              </div>
            );
          })}
        </div>

        {extensions.length === 0 && (
          <p className="text-center text-muted-foreground mt-4 text-sm">
            No wallet extensions detected. Install one to get started.
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function AccountList() {
  const { accounts, status, connectedExtension } = useWalletState();
  const selectedAccount = useSelectedAccount();
  const [copied, setCopied] = useState<string | null>(null);

  const copyAddress = async (address: string) => {
    await navigator.clipboard.writeText(address);
    setCopied(address);
    setTimeout(() => setCopied(null), 2000);
  };

  if (status !== "connected" || !connectedExtension) {
    return null;
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle>Connected Accounts</CardTitle>
            <CardDescription>
              {accounts.length} account{accounts.length !== 1 ? "s" : ""} from{" "}
              {connectedExtension.name}
            </CardDescription>
          </div>
          <Button variant="outline" size="sm" onClick={disconnectWallet}>
            <LogOut className="h-4 w-4 mr-2" />
            Disconnect
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {accounts.length === 0 ? (
          <p className="text-center text-muted-foreground py-4">
            No accounts found. Make sure you have accounts in your wallet.
          </p>
        ) : (
          <div className="space-y-2">
            {accounts.map((account) => {
              const isSelected = selectedAccount?.address === account.address;

              return (
                <div
                  key={account.address}
                  className={cn(
                    "flex items-center justify-between p-3 rounded-md border cursor-pointer transition-colors",
                    isSelected
                      ? "border-primary bg-primary/5"
                      : "hover:bg-secondary/50"
                  )}
                  onClick={() => selectAccount(account.address)}
                >
                  <div className="flex items-center gap-3 min-w-0">
                    <div
                      className={cn(
                        "w-10 h-10 rounded-full flex items-center justify-center text-lg font-bold",
                        isSelected ? "bg-primary text-white" : "bg-secondary"
                      )}
                    >
                      {(account.name || "?")[0]?.toUpperCase()}
                    </div>
                    <div className="min-w-0">
                      <p className="font-medium truncate">
                        {account.name || "Unnamed Account"}
                      </p>
                      <p className="text-sm text-muted-foreground font-mono truncate">
                        {formatAddress(account.address, 8)}
                      </p>
                    </div>
                  </div>

                  <div className="flex items-center gap-2 ml-2">
                    {isSelected && (
                      <Badge variant="secondary">Selected</Badge>
                    )}
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={(e) => {
                        e.stopPropagation();
                        copyAddress(account.address);
                      }}
                    >
                      {copied === account.address ? (
                        <Check className="h-4 w-4 text-success" />
                      ) : (
                        <Copy className="h-4 w-4" />
                      )}
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function SelectedAccountDetails() {
  const selectedAccount = useSelectedAccount();
  const [copied, setCopied] = useState(false);

  if (!selectedAccount) {
    return null;
  }

  const copyAddress = async () => {
    await navigator.clipboard.writeText(selectedAccount.address);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Selected Account</CardTitle>
        <CardDescription>
          This account will be used for all transactions
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex items-center gap-4">
          <div className="w-16 h-16 rounded-full bg-primary flex items-center justify-center text-2xl font-bold text-white">
            {(selectedAccount.name || "?")[0]?.toUpperCase()}
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-xl font-semibold truncate">
              {selectedAccount.name || "Unnamed Account"}
            </p>
            <p className="text-sm text-muted-foreground">
              Type: {selectedAccount.type || "Unknown"}
            </p>
          </div>
        </div>

        <div className="p-3 rounded-md bg-secondary">
          <p className="text-xs text-muted-foreground mb-1">Address</p>
          <div className="flex items-center gap-2">
            <p className="font-mono text-sm break-all flex-1">
              {selectedAccount.address}
            </p>
            <Button variant="ghost" size="icon" onClick={copyAddress}>
              {copied ? (
                <Check className="h-4 w-4 text-success" />
              ) : (
                <Copy className="h-4 w-4" />
              )}
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

export function Accounts() {
  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Accounts</h1>
        <p className="text-muted-foreground">
          Manage your wallet connections and accounts
        </p>
      </div>

      <div className="grid gap-6 lg:grid-cols-2">
        <div className="space-y-6">
          <ExtensionList />
          <AccountList />
        </div>
        <div>
          <SelectedAccountDetails />
        </div>
      </div>
    </div>
  );
}
