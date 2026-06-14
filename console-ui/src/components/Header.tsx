// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

import { Link, useLocation } from "react-router-dom";
import { Database, Upload, Download, RefreshCw, Search, Shield, Wallet, Menu, AlertTriangle, HelpCircle, BookOpen, ExternalLink, ChevronDown, X } from "lucide-react";

// Brand icons were removed in lucide-react 1.x — inline the Github icon SVG from 0.577.0
const GithubIcon = ({ className }: { className?: string }) => (
  <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className={className}>
    <path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.403 5.403 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4" />
    <path d="M9 18c-4.51 2-5-2-7-2" />
  </svg>
);
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
  getCustomNetworkUrl,
  clearCustomNetworkUrl,
  type NetworkId,
} from "@/state/chain.state";
import { WEB3_STORAGE_URL } from "@/config/networks";
import { useWalletState, useSelectedAccount } from "@/state/wallet.state";
import { useAuthorization, useAuthorizationLoading } from "@/state/storage.state";
import { formatAddress, formatBlockNumber } from "@/utils/format";
import { cn } from "@/utils/cn";
import React, { useState, useEffect } from "react";

// All navigation items
const navItems = [
  { path: "/", label: "Dashboard", icon: Database, requiresAuth: false },
  { path: "/authorizations", label: "Faucet", icon: Shield, requiresAuth: false },
  { path: "/explorer", label: "Explorer", icon: Search, requiresAuth: false },
  { path: "/upload", label: "Upload", icon: Upload, requiresAuth: true },
  { path: "/download", label: "Download", icon: Download, requiresAuth: false },
  { path: "/renew", label: "Renew", icon: RefreshCw, requiresAuth: true },
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
  const { blockNumber } = useChainState();

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

  const helpLinks = [
    {
      label: "User Manual",
      href: `${import.meta.env.BASE_URL}docs/index.html`,
      icon: BookOpen,
      external: true,
      description: "Guides for storing and retrieving data",
    },
    {
      label: "GitHub Repository",
      href: "https://github.com/paritytech/polkadot-bulletin-chain",
      icon: GithubIcon,
      external: true,
      description: "View source code and contribute",
    },
  ];

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

      {open && (
        <div className="absolute right-0 top-full mt-2 w-64 rounded-md border bg-popover p-2 shadow-lg z-50">
            <div className="text-xs font-medium text-muted-foreground px-2 py-1.5">
              Help & Resources
            </div>
            {helpLinks.map((link) => (
              <a
                key={link.label}
                href={link.href}
                target={link.external ? "_blank" : "_self"}
                rel={link.external ? "noopener noreferrer" : undefined}
                className="flex items-start gap-3 rounded-sm px-2 py-2 hover:bg-accent transition-colors"
                onClick={() => setOpen(false)}
              >
                <link.icon className="h-4 w-4 mt-0.5 text-muted-foreground" />
                <div className="flex-1">
                  <div className="flex items-center gap-1 text-sm font-medium">
                    {link.label}
                    {link.external && <ExternalLink className="h-3 w-3" />}
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
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const { status, network } = useChainState();
  const selectedAccount = useSelectedAccount();
  const authorization = useAuthorization();

  // Auto-connect on mount using the persisted network selection
  useEffect(() => {
    if (status === "disconnected") {
      connectToNetwork(network.id);
    }
  }, []);

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
            <Button
              variant="ghost"
              size="sm"
              asChild
              className="h-8 text-xs hidden sm:inline-flex"
            >
              <a href={WEB3_STORAGE_URL} target="_blank" rel="noopener noreferrer">
                Web3 Storage
                <ExternalLink className="h-3 w-3 ml-1.5" />
              </a>
            </Button>
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
            const disabled = item.requiresAuth && (!selectedAccount || !authorization);
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
                const disabled = item.requiresAuth && (!selectedAccount || !authorization);
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
