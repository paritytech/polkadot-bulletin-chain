// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

import { Link } from "react-router-dom";
import { SS58String } from "polkadot-api";
import { AlertCircle, RefreshCw, User } from "lucide-react";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/Card";
import { Spinner } from "@/components/ui/Spinner";
import { PalletUnavailableNotice } from "@/components/PalletUnavailableNotice";
import { useApi } from "@/state/chain.state";
import {
  fetchAccountAuthorization,
  useAuthorization,
  useAuthorizationError,
  useAuthorizationLoading,
} from "@/state/storage.state";
import { useSelectedAccount } from "@/state/wallet.state";
import { formatAddress, formatBytes, formatNumber } from "@/utils/format";

export function AccountSummaryCard({ className }: { className?: string }) {
  const selectedAccount = useSelectedAccount();
  const authorization = useAuthorization();
  const isLoading = useAuthorizationLoading();
  const error = useAuthorizationError();
  const api = useApi();

  const handleRefresh = () => {
    if (api && selectedAccount) {
      fetchAccountAuthorization(api, selectedAccount.address as SS58String);
    }
  };

  if (!selectedAccount) {
    return (
      <Card className={className}>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <User className="h-5 w-5" />
            Your Account
          </CardTitle>
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
    <Card className={className}>
      <CardHeader>
        <div className="flex items-start justify-between">
          <div>
            <CardTitle className="flex items-center gap-2">
              <User className="h-5 w-5" />
              Your Account
            </CardTitle>
            <CardDescription>{selectedAccount.name || "Unknown"}</CardDescription>
          </div>
          <Button
            variant="ghost"
            size="icon"
            onClick={handleRefresh}
            disabled={isLoading || !api}
            aria-label="Refresh authorization"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        <div className="space-y-4">
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

          <hr />

          {isLoading ? (
            <div className="flex items-center justify-center h-20">
              <Spinner />
            </div>
          ) : error ? (
            <PalletUnavailableNotice pallet="TransactionStorage" details={error} />
          ) : authorization ? (
            <>
              <div>
                <p className="text-sm font-medium mb-2">Storage Used</p>
                <div className="grid grid-cols-3 gap-4">
                  <Stat label="Transactions" value={formatNumber(authorization.used.transactions)} />
                  <Stat label="Ephemeral" value={formatBytes(authorization.used.bytesEphemeral)} />
                  <Stat label="Permanent" value={formatBytes(authorization.used.bytesPermanent)} />
                </div>
              </div>

              <hr />

              <div>
                <p className="text-sm font-medium mb-2">Authorization</p>
                <div className="grid grid-cols-2 gap-4">
                  <Stat label="Transactions" value={formatNumber(authorization.allowance.transactions)} />
                  <Stat label="Bytes" value={formatBytes(authorization.allowance.bytes)} />
                </div>
                {authorization.expiresAt && (
                  <div className="mt-3 flex items-center justify-between">
                    <span className="text-sm text-muted-foreground">Expires at block</span>
                    <Badge variant="secondary">#{authorization.expiresAt}</Badge>
                  </div>
                )}
              </div>
            </>
          ) : (
            <div className="flex flex-col items-center justify-center h-20 text-muted-foreground gap-2">
              <AlertCircle className="h-5 w-5" />
              <p className="text-sm">No authorization found for this account</p>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="space-y-1">
      <p className="text-xs text-muted-foreground uppercase tracking-wide">{label}</p>
      <p className="text-lg font-semibold">{value}</p>
    </div>
  );
}
