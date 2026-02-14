import { Link, useLocation } from "react-router-dom";
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
  NETWORKS,
  type NetworkId,
} from "@/state/chain.state";
import { useWalletState, useSelectedAccount } from "@/state/wallet.state";
import { formatAddress, formatBlockNumber } from "@/utils/format";
import { cn } from "@/utils/cn";
import { useState, useEffect } from "react";

const navItems = [
  { path: "/", label: "Dashboard", icon: Database },
  { path: "/upload", label: "Upload", icon: Upload },
  { path: "/download", label: "Download", icon: Download },
  { path: "/explorer", label: "Explorer", icon: Search },
  { path: "/authorizations", label: "Auth", icon: Shield },
  { path: "/accounts", label: "Accounts", icon: Wallet },
];

function ConnectionStatus() {
  const { status, blockNumber, network } = useChainState();

  const statusColors = {
    disconnected: "bg-gray-500",
    connecting: "bg-yellow-500 animate-pulse",
    connected: "bg-green-500",
    error: "bg-red-500",
  };

  return (
    <div className="flex items-center gap-2 text-sm">
      <div className={cn("w-2 h-2 rounded-full", statusColors[status])} />
      <span className="hidden sm:inline text-muted-foreground">
        {network.name}
      </span>
      {status === "connected" && blockNumber !== undefined && (
        <Badge variant="secondary" className="font-mono text-xs">
          {formatBlockNumber(blockNumber)}
        </Badge>
      )}
    </div>
  );
}

function NetworkSwitcher() {
  const { network, status } = useChainState();

  const handleNetworkChange = (value: string) => {
    connectToNetwork(value as NetworkId);
  };

  return (
    <Select value={network.id} onValueChange={handleNetworkChange}>
      <SelectTrigger className="w-[140px]" disabled={status === "connecting"}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {Object.values(NETWORKS).map((net) => (
          <SelectItem key={net.id} value={net.id}>
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
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const { status } = useChainState();

  // Auto-connect on mount
  useEffect(() => {
    if (status === "disconnected") {
      connectToNetwork("local");
    }
  }, []);

  return (
    <header className="sticky top-0 z-40 border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
      <div className="container mx-auto max-w-7xl px-4">
        <div className="flex h-14 items-center justify-between">
          {/* Logo */}
          <Link to="/" className="flex items-center gap-2">
            <div className="w-8 h-8 rounded-lg bg-primary flex items-center justify-center">
              <span className="text-white font-bold text-lg">B</span>
            </div>
            <span className="font-semibold hidden sm:inline">Bulletin Console</span>
          </Link>

          {/* Desktop Navigation */}
          <nav className="hidden md:flex items-center gap-1">
            {navItems.map(({ path, label, icon: Icon }) => (
              <Link key={path} to={path}>
                <Button
                  variant={location.pathname === path ? "secondary" : "ghost"}
                  size="sm"
                >
                  <Icon className="h-4 w-4 mr-1" />
                  {label}
                </Button>
              </Link>
            ))}
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
              {navItems.map(({ path, label, icon: Icon }) => (
                <Link
                  key={path}
                  to={path}
                  onClick={() => setMobileMenuOpen(false)}
                >
                  <Button
                    variant={location.pathname === path ? "secondary" : "ghost"}
                    className="w-full justify-start"
                  >
                    <Icon className="h-4 w-4 mr-2" />
                    {label}
                  </Button>
                </Link>
              ))}
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
