import { Link, useLocation, useNavigate } from "react-router-dom";
import { Database, Upload, Download, RefreshCw, Search, Shield, Wallet, Menu, AlertTriangle, HelpCircle, BookOpen, ExternalLink, ChevronDown, X, Activity, Globe, LineChart, BarChart3 } from "lucide-react";

import { Button } from "@/components/ui/Button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select";
import { Badge } from "@/components/ui/Badge";
import * as SelectPrimitive from "@radix-ui/react-select";
import {
  useChainState,
  connectToNetwork,
  switchStorageType,
  STORAGE_CONFIGS,
  getCustomNetworkUrl,
  clearCustomNetworkUrl,
  type NetworkId,
  type StorageType,
} from "@/state/chain.state";
import { useWalletState, useSelectedAccount } from "@/state/wallet.state";
import { useAuthorization, useAuthorizationLoading } from "@/state/storage.state";
import { formatAddress, formatBlockNumber } from "@/utils/format";
import { cn } from "@/utils/cn";
import React, { useState, useEffect } from "react";

// All navigation items
const navItems = [
  { path: "/", label: "Dashboard", icon: Database, web3storage: true, requiresAuth: false },
  { path: "/authorizations", label: "Faucet", icon: Shield, web3storage: false, requiresAuth: false },
  { path: "/explorer", label: "Explorer", icon: Search, web3storage: true, requiresAuth: false },
  { path: "/upload", label: "Upload", icon: Upload, web3storage: false, requiresAuth: true },
  { path: "/download", label: "Download", icon: Download, web3storage: false, requiresAuth: false },
  { path: "/renew", label: "Renew", icon: RefreshCw, web3storage: false, requiresAuth: true },
  { path: "/ops", label: "Ops", icon: Activity, web3storage: true, requiresAuth: false },
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
        <Badge variant="secondary" className="font-mono text-xs" data-testid="block-number">
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
  const isCustom = network.id === "custom";
  const activeCustomUrl = isCustom ? network.endpoints[0] : undefined;
  const [customUrl, setCustomUrl] = useState(() => activeCustomUrl ?? getCustomNetworkUrl());

  useEffect(() => {
    if (isCustom && activeCustomUrl) setCustomUrl(activeCustomUrl);
  }, [isCustom, activeCustomUrl]);

  const handleNetworkChange = (value: string) => {
    if (value === "custom") {
      const saved = getCustomNetworkUrl();
      connectToNetwork("custom", saved || undefined);
      if (saved) setCustomUrl(saved);
      return;
    }
    connectToNetwork(value as NetworkId);
  };

  const handleCustomConnect = () => {
    const url = customUrl.trim();
    if (!url || url === activeCustomUrl) return;
    connectToNetwork("custom", url);
  };

  const handleClearCustom = () => {
    setCustomUrl("");
    clearCustomNetworkUrl();
  };

  const selectItems = Object.values(networks).map((net) => (
    <SelectItem
      key={net.id}
      value={net.id}
      disabled={net.id !== "custom" && net.endpoints.length === 0}
    >
      {net.name}
    </SelectItem>
  ));

  if (!isCustom) {
    return (
      <Select value={network.id} onValueChange={handleNetworkChange}>
        <SelectTrigger className="w-[260px]">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>{selectItems}</SelectContent>
      </Select>
    );
  }

  // In custom mode the trigger collapses into a single 260px-wide pill:
  // [ wss URL input | ✕ | ▾ ]. The chevron is the Radix SelectTrigger so the
  // dropdown still works to switch back to a preset network.
  const dirty = customUrl.trim() !== "" && customUrl.trim() !== activeCustomUrl;
  return (
    <Select value={network.id} onValueChange={handleNetworkChange}>
      <div className="flex h-10 w-[260px] items-center rounded-md border-2 border-primary/60 bg-background pl-2 pr-1 ring-offset-background focus-within:border-primary focus-within:ring-2 focus-within:ring-primary/30 focus-within:ring-offset-2">
        <input
          value={customUrl}
          onChange={(e) => setCustomUrl(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              handleCustomConnect();
            }
          }}
          placeholder="wss://…"
          className="min-w-0 flex-1 bg-transparent text-sm font-mono placeholder:text-muted-foreground focus:outline-none"
        />
        {dirty && (
          <button
            type="button"
            onClick={handleCustomConnect}
            className="ml-1 rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-primary hover:bg-primary/10"
            title="Connect to this URL"
          >
            Go
          </button>
        )}
        {(activeCustomUrl || customUrl) && (
          <button
            type="button"
            onClick={handleClearCustom}
            className="ml-1 rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
            title="Remove custom URL"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
        <SelectPrimitive.Trigger
          className="ml-1 inline-flex h-7 w-7 items-center justify-center rounded hover:bg-muted focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
          title="Pick another network"
        >
          <ChevronDown className="h-4 w-4 opacity-70" />
        </SelectPrimitive.Trigger>
      </div>
      <SelectContent>{selectItems}</SelectContent>
    </Select>
  );
}

function HelpMenu() {
  const [open, setOpen] = useState(false);
  const menuRef = React.useRef<HTMLDivElement>(null);

  // Close menu when clicking outside
  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    }

    if (open) {
      document.addEventListener("mousedown", handleClickOutside);
      return () => document.removeEventListener("mousedown", handleClickOutside);
    }
  }, [open]);

  const { network } = useChainState();
  const monitoring = network?.monitoring;

  const monitoringLinks = monitoring
    ? [
        monitoring.grafana && {
          label: "Grafana (Operation Health)",
          href: monitoring.grafana,
          icon: Activity,
          description: "Block production, finality, peers",
        },
        monitoring.sentry && {
          label: "Sentry (Bulletin Deploy Health)",
          href: monitoring.sentry,
          icon: LineChart,
          description: "Product-side write latency and failures",
        },
        monitoring.sentryStorageSpan && {
          label: "Sentry: deploy.storage",
          href: monitoring.sentryStorageSpan,
          icon: LineChart,
          description: "Per-deploy total Bulletin write time",
        },
        monitoring.sentryChunkUploadSpan && {
          label: "Sentry: deploy.chunk-upload",
          href: monitoring.sentryChunkUploadSpan,
          icon: LineChart,
          description: "Per-chunk submit-to-finalized latency",
        },
        monitoring.sentryChainProbeSpan && {
          label: "Sentry: deploy.chain-probe",
          href: monitoring.sentryChainProbeSpan,
          icon: LineChart,
          description: "Cache-check RPC reads against the chain",
        },
        monitoring.telemetry && {
          label: "Operator Set",
          href: monitoring.telemetry,
          icon: Globe,
          description: "Live list of every node running this chain",
        },
        monitoring.polkadotJs && {
          label: "PolkadotJS Apps",
          href: monitoring.polkadotJs,
          icon: ExternalLink,
          description: "Inspect chain state and events",
        },
        monitoring.explorer && {
          label: "Block Explorer",
          href: monitoring.explorer,
          icon: BarChart3,
          description: "Browse blocks and extrinsics",
        },
        monitoring.runbook && {
          label: "Runbook",
          href: monitoring.runbook,
          icon: BookOpen,
          description: "Operational playbook",
        },
      ].filter((x): x is Exclude<typeof x, false | undefined | ""> => Boolean(x))
    : [];

  return (
    <div className="relative" ref={menuRef}>
      <Button
        variant="ghost"
        size="icon"
        onClick={() => setOpen(!open)}
        className="h-8 w-8"
      >
        <HelpCircle className="h-4 w-4" />
      </Button>

      {open && monitoringLinks.length > 0 && (
        <div className="absolute right-0 top-full mt-2 w-80 rounded-md border bg-popover p-2 shadow-lg z-50">
            <div className="text-xs font-medium text-muted-foreground px-2 py-1.5">
              Monitoring &amp; Diagnostics
            </div>
            {monitoringLinks.map((link) => (
              <a
                key={link.label}
                href={link.href}
                target="_blank"
                rel="noopener noreferrer"
                className="flex items-start gap-3 rounded-sm px-2 py-2 hover:bg-accent transition-colors"
                onClick={() => setOpen(false)}
              >
                <link.icon className="h-4 w-4 mt-0.5 text-muted-foreground" />
                <div className="flex-1">
                  <div className="flex items-center gap-1 text-sm font-medium">
                    {link.label}
                    <ExternalLink className="h-3 w-3" />
                  </div>
                  <div className="text-xs text-muted-foreground">
                    {link.description}
                  </div>
                </div>
              </a>
            ))}
          </div>
      )}
    </div>
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
            <HelpMenu />
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
