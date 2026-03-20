import { Link, useLocation, useNavigate } from "react-router-dom";
import { Database, Upload, Download, RefreshCw, Search, Shield, Wallet, Menu, AlertTriangle } from "lucide-react";
import { Button } from "@/components/ui/Button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select";
import { Badge } from "@/components/ui/Badge";
import {
  useChainState,
  connectToNetwork,
  switchStorageType,
  STORAGE_CONFIGS,
  type NetworkId,
  type StorageType,
} from "@/state/chain.state";
import { useWalletState, useSelectedAccount } from "@/state/wallet.state";
import { useAuthorization, useAuthorizationLoading } from "@/state/storage.state";
import { formatAddress, formatBlockNumber } from "@/utils/format";
import { cn } from "@/utils/cn";
import { useState, useEffect } from "react";

// All navigation items
const navItems = [
  { path: "/", label: "Dashboard", icon: Database, web3storage: true, requiresAuth: false },
  { path: "/authorizations", label: "Faucet", icon: Shield, web3storage: false, requiresAuth: false },
  { path: "/explorer", label: "Explorer", icon: Search, web3storage: true, requiresAuth: false },
  { path: "/upload", label: "Upload", icon: Upload, web3storage: false, requiresAuth: true },
  { path: "/download", label: "Download", icon: Download, web3storage: false, requiresAuth: false },
  { path: "/renew", label: "Renew", icon: RefreshCw, web3storage: false, requiresAuth: true },
] as const;

function ConnectionStatus() {
  const { status, blockNumber } = useChainState();

  const statusColors = {
    disconnected: "bg-gray-500",
    connecting: "bg-yellow-500 animate-pulse",
    connected: "bg-green-500",
    error: "bg-red-500",
  };

  return (
    <div className="flex items-center gap-2 text-sm">
      <div className={cn("w-2 h-2 rounded-full", statusColors[status])} />
      {status === "connected" && blockNumber !== undefined && (
        <Badge variant="secondary" className="font-mono text-xs">
          {formatBlockNumber(blockNumber)}
        </Badge>
      )}
    </div>
  );
}

function AuthorizationStatus() {
  const selectedAccount = useSelectedAccount();
  const authorization = useAuthorization();
  const isLoading = useAuthorizationLoading();
  const { blockNumber, storageType } = useChainState();

  // Don't show for web3storage mode
  if (storageType === "web3storage") {
    return null;
  }

  // Not connected - don't show anything (Connect button already visible)
  if (!selectedAccount) {
    return null;
  }

  // Loading
  if (isLoading) {
    return (
      <div className="hidden lg:flex items-center gap-2 px-3 py-1 rounded-md bg-muted/50 text-xs">
        <span className="text-muted-foreground">Loading...</span>
      </div>
    );
  }

  // No authorization
  if (!authorization) {
    return (
      <Link to="/authorizations">
        <div className="hidden lg:flex items-center gap-2 px-3 py-1 rounded-md bg-destructive/10 text-xs text-destructive hover:bg-destructive/20 transition-colors cursor-pointer">
          <AlertTriangle className="h-3 w-3" />
          <span>No authorization - Get from Faucet</span>
        </div>
      </Link>
    );
  }

  // Calculate blocks until expiry
  const blocksUntilExpiry = authorization.expiresAt && blockNumber !== undefined
    ? authorization.expiresAt - blockNumber
    : null;
  const isExpiringSoon = blocksUntilExpiry !== null && blocksUntilExpiry > 0 && blocksUntilExpiry < 1000;
  const isExpired = blocksUntilExpiry !== null && blocksUntilExpiry <= 0;

  // Has authorization - simple indicator
  return (
    <div className={cn(
      "hidden lg:flex items-center gap-2 px-3 py-1 rounded-md text-xs",
      isExpired ? "bg-destructive/10 text-destructive" : isExpiringSoon ? "bg-yellow-500/10 text-yellow-600" : "bg-green-500/10 text-green-600"
    )}>
      <Shield className="h-3 w-3" />
      <span className="font-medium">
        {isExpired ? "Authorization Expired" : "Authorized"}
      </span>
    </div>
  );
}

function NetworkSwitcher() {
  const { network, networks } = useChainState();

  const handleNetworkChange = (value: string) => {
    connectToNetwork(value as NetworkId);
  };

  return (
    <Select value={network.id} onValueChange={handleNetworkChange}>
      <SelectTrigger className="w-[260px]">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {Object.values(networks).map((net) => (
          <SelectItem key={net.id} value={net.id} disabled={net.endpoints.length === 0}>
            {net.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function AccountDisplay() {
  const selectedAccount = useSelectedAccount();
  const { status } = useWalletState();

  if (status !== "connected" || !selectedAccount) {
    return (
      <Link to="/accounts">
        <Button variant="outline" size="sm">
          <Wallet className="h-4 w-4 mr-2" />
          Connect
        </Button>
      </Link>
    );
  }

  return (
    <Link to="/accounts">
      <Button variant="ghost" size="sm" className="font-mono">
        {formatAddress(selectedAccount.address, 4)}
      </Button>
    </Link>
  );
}

export function Header() {
  const location = useLocation();
  const navigate = useNavigate();
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const { status, storageType, network } = useChainState();
  const selectedAccount = useSelectedAccount();
  const authorization = useAuthorization();

  // Auto-connect on mount using the persisted network selection
  useEffect(() => {
    if (status === "disconnected") {
      connectToNetwork(network.id);
    }
  }, []);

  // Redirect to Dashboard if current route is disabled for the active storage type
  useEffect(() => {
    const currentItem = navItems.find((item) => item.path === location.pathname);
    if (currentItem && storageType === "web3storage" && !currentItem.web3storage) {
      navigate("/");
    }
  }, [storageType, location.pathname, navigate]);

  return (
    <header className="sticky top-0 z-40 border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
      <div className="container mx-auto max-w-7xl px-4">
        {/* Top Row: Meta Information */}
        <div className="flex h-12 items-center justify-between border-b border-border/50">
          {/* Logo */}
          <div className="flex items-center gap-3">
            <Link to="/" className="flex items-center gap-2">
              <div className="w-7 h-7 rounded-lg bg-primary flex items-center justify-center">
                <span className="text-white font-bold">B</span>
              </div>
              <span className="font-semibold hidden sm:inline">Bulletin Chain</span>
            </Link>
            <AuthorizationStatus />
          </div>

          {/* Network, Status, Account */}
          <div className="flex items-center gap-3">
            <ConnectionStatus />
            <div className="hidden sm:block">
              <NetworkSwitcher />
            </div>
            <Select value={storageType} onValueChange={(v) => switchStorageType(v as StorageType)}>
              <SelectTrigger className="w-[130px] h-8 text-xs hidden sm:flex">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {Object.values(STORAGE_CONFIGS).map((config) => (
                  <SelectItem key={config.id} value={config.id}>
                    {config.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <AccountDisplay />

            {/* Mobile menu button */}
            <Button
              variant="ghost"
              size="icon"
              className="md:hidden h-8 w-8"
              onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
            >
              <Menu className="h-4 w-4" />
            </Button>
          </div>
        </div>

        {/* Bottom Row: Navigation */}
        <nav className="hidden md:flex items-center gap-1 h-10">
          {navItems.map((item) => {
            const disabledByStorageType = storageType === "web3storage" && !item.web3storage;
            const disabledByAuth = item.requiresAuth && (!selectedAccount || !authorization);
            const disabled = disabledByStorageType || disabledByAuth;
            if (disabled) {
              return (
                <Button
                  key={item.path}
                  variant="ghost"
                  size="sm"
                  disabled
                  className="opacity-30 h-8"
                >
                  <item.icon className="h-4 w-4 mr-1.5" />
                  {item.label}
                </Button>
              );
            }
            return (
              <Link key={item.path} to={item.path}>
                <Button
                  variant={location.pathname === item.path ? "default" : "ghost"}
                  size="sm"
                  className="h-8"
                >
                  <item.icon className="h-4 w-4 mr-1.5" />
                  {item.label}
                </Button>
              </Link>
            );
          })}
        </nav>

        {/* Mobile Navigation */}
        {mobileMenuOpen && (
          <nav className="md:hidden py-4 border-t">
            <div className="flex flex-col gap-1">
              {navItems.map((item) => {
                const disabledByStorageType = storageType === "web3storage" && !item.web3storage;
                const disabledByAuth = item.requiresAuth && (!selectedAccount || !authorization);
                const disabled = disabledByStorageType || disabledByAuth;
                if (disabled) {
                  return (
                    <Button
                      key={item.path}
                      variant="ghost"
                      className="w-full justify-start opacity-30"
                      disabled
                    >
                      <item.icon className="h-4 w-4 mr-2" />
                      {item.label}
                    </Button>
                  );
                }
                return (
                  <Link
                    key={item.path}
                    to={item.path}
                    onClick={() => setMobileMenuOpen(false)}
                  >
                    <Button
                      variant={location.pathname === item.path ? "default" : "ghost"}
                      className="w-full justify-start"
                    >
                      <item.icon className="h-4 w-4 mr-2" />
                      {item.label}
                    </Button>
                  </Link>
                );
              })}
              <div className="pt-2 mt-2 border-t">
                <NetworkSwitcher />
              </div>
            </div>
          </nav>
        )}
      </div>
    </header>
  );
}
