import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import { RefreshCw, Shield, AlertCircle } from "lucide-react";
import { useAuthorization, useAuthorizationLoading, useAuthorizationError, fetchAccountAuthorization } from "@/state/storage.state";
import { useApi } from "@/state/chain.state";
import { useSelectedAccount } from "@/state/wallet.state";
import { formatBytes, formatNumber } from "@/utils/format";
import { SS58String } from "polkadot-api";

export function AuthorizationCard() {
  const authorization = useAuthorization();
  const isLoading = useAuthorizationLoading();
  const error = useAuthorizationError();
  const api = useApi();
  const selectedAccount = useSelectedAccount();

  const handleRefresh = () => {
    if (api && selectedAccount) {
      fetchAccountAuthorization(api, selectedAccount.address as SS58String);
    }
  };

  if (!selectedAccount) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Shield className="h-5 w-5" />
            Authorization Status
          </CardTitle>
          <CardDescription>Connect a wallet to view authorization</CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-center h-20 text-muted-foreground">
            No wallet connected
          </div>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle className="flex items-center gap-2">
              <Shield className="h-5 w-5" />
              Authorization Status
            </CardTitle>
            <CardDescription>Your storage quota and permissions</CardDescription>
          </div>
          <Button
            variant="ghost"
            size="icon"
            onClick={handleRefresh}
            disabled={isLoading || !api}
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="flex items-center justify-center h-20">
            <Spinner />
          </div>
        ) : error ? (
          <div className="flex items-center gap-2 text-destructive">
            <AlertCircle className="h-4 w-4" />
            <span className="text-sm">{error}</span>
          </div>
        ) : authorization ? (
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-1">
                <p className="text-xs text-muted-foreground uppercase tracking-wide">
                  Transactions
                </p>
                <p className="text-2xl font-semibold">
                  {formatNumber(authorization.transactions)}
                </p>
              </div>
              <div className="space-y-1">
                <p className="text-xs text-muted-foreground uppercase tracking-wide">
                  Bytes
                </p>
                <p className="text-2xl font-semibold">
                  {formatBytes(authorization.bytes)}
                </p>
              </div>
            </div>
            {authorization.expiresAt && (
              <div className="pt-2 border-t">
                <div className="flex items-center justify-between">
                  <span className="text-sm text-muted-foreground">Expires at block</span>
                  <Badge variant="secondary">#{authorization.expiresAt}</Badge>
                </div>
              </div>
            )}
          </div>
        ) : (
          <div className="flex flex-col items-center justify-center h-20 text-muted-foreground gap-2">
            <AlertCircle className="h-5 w-5" />
            <p className="text-sm">No authorization found for this account</p>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
