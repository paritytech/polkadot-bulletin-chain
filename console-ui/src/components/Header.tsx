import { Link, useLocation, useNavigate } from "react-router-dom";
import { Database, Upload, Download, Search, Shield, Wallet, Menu } from "lucide-react";
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
import { formatAddress, formatBlockNumber } from "@/utils/format";
import { cn } from "@/utils/cn";
import { useState, useEffect } from "react";

const navItems = [
  { path: "/", label: "Dashboard", icon: Database, web3storage: true },
  { path: "/authorizations", label: "Faucet", icon: Shield, web3storage: false },
  { path: "/explorer", label: "Explorer", icon: Search, web3storage: true },
  { path: "/upload", label: "Upload", icon: Upload, web3storage: false },
  { path: "/download", label: "Download", icon: Download, web3storage: false },
  { separator: true },
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

  // Auto-connect on mount using the persisted network selection
  useEffect(() => {
    if (status === "disconnected") {
      connectToNetwork(network.id);
    }
  }, []);

  // Redirect to Dashboard if current route is disabled for the active storage type
  useEffect(() => {
    const currentItem = navItems.find(
      (item) => !("separator" in item) && item.path === location.pathname
    );
    if (currentItem && !("separator" in currentItem) && storageType === "web3storage" && !currentItem.web3storage) {
      navigate("/");
    }
  }, [storageType, location.pathname, navigate]);

  return (
    <header className="sticky top-0 z-40 border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
      <div className="container mx-auto max-w-7xl px-4">
        <div className="flex h-14 items-center justify-between">
          {/* Logo & Storage Type */}
          <div className="flex items-center gap-3">
            <Link to="/" className="flex items-center gap-2">
              <div className="w-8 h-8 rounded-lg bg-primary flex items-center justify-center">
                <span className="text-white font-bold text-lg">S</span>
              </div>
              <span className="font-semibold hidden sm:inline">Storage Console</span>
            </Link>
            <Select value={storageType} onValueChange={(v) => switchStorageType(v as StorageType)}>
              <SelectTrigger className="w-[140px] hidden sm:flex">
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
            <div className="w-px h-5 bg-border hidden sm:block" />
          </div>

          {/* Desktop Navigation */}
          <nav className="hidden md:flex items-center gap-1">
            {navItems.map((item, i) => {
              if ("separator" in item) {
                return <div key={i} className="w-px h-5 bg-border mx-1" />;
              }
              const disabled = storageType === "web3storage" && !item.web3storage;
              if (disabled) {
                return (
                  <Button
                    key={item.path}
                    variant="ghost"
                    size="sm"
                    disabled
                    className="opacity-30"
                  >
                    <item.icon className="h-4 w-4 mr-1" />
                    {item.label}
                  </Button>
                );
              }
              return (
                <Link key={item.path} to={item.path}>
                  <Button
                    variant={location.pathname === item.path ? "default" : "ghost"}
                    size="sm"
                  >
                    <item.icon className="h-4 w-4 mr-1" />
                    {item.label}
                  </Button>
                </Link>
              );
            })}
          </nav>

          {/* Right side */}
          <div className="flex items-center gap-2">
            <ConnectionStatus />
            <div className="hidden sm:block">
              <NetworkSwitcher />
            </div>
            <AccountDisplay />

            {/* Mobile menu button */}
            <Button
              variant="ghost"
              size="icon"
              className="md:hidden"
              onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
            >
              <Menu className="h-5 w-5" />
            </Button>
          </div>
        </div>

        {/* Mobile Navigation */}
        {mobileMenuOpen && (
          <nav className="md:hidden py-4 border-t">
            <div className="flex flex-col gap-1">
              {navItems.map((item, i) => {
                if ("separator" in item) {
                  return <div key={i} className="h-px bg-border my-1" />;
                }
                const disabled = storageType === "web3storage" && !item.web3storage;
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
