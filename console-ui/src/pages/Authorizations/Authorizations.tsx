import { useState, useEffect } from "react";
import { RefreshCw, User, FileText, AlertCircle, Search, Plus, Shield, Droplet } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/Tabs";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/Dialog";
import { useApi } from "@/state/chain.state";
import { useSudoKey } from "@/state/chain.state";
import { useSelectedAccount } from "@/state/wallet.state";
import {
  useAuthorization,
  useAuthorizationLoading,
  usePreimageAuthorizations,
  usePreimageAuthsLoading,
  fetchAccountAuthorization,
  fetchPreimageAuthorizations,
} from "@/state/storage.state";
import { formatBytes, formatAddress, bytesToHex } from "@/utils/format";
import { SS58String, Enum, Binary } from "polkadot-api";
import { cryptoWaitReady } from "@polkadot/util-crypto";
import { Keyring } from "@polkadot/keyring";
import { getPolkadotSigner } from "polkadot-api/signer";

interface AuthorizeAccountFormProps {
  onSubmit: (address: string, transactions: bigint, bytes: bigint) => Promise<void>;
  isSubmitting: boolean;
}

function AuthorizeAccountForm({ onSubmit, isSubmitting }: AuthorizeAccountFormProps) {
  const [address, setAddress] = useState("");
  const [transactions, setTransactions] = useState("");
  const [bytes, setBytes] = useState("");
  const [bytesUnit, setBytesUnit] = useState<"B" | "KB" | "MB">("KB");

  const getBytesValue = (): bigint => {
    const value = parseInt(bytes, 10);
    if (isNaN(value)) return 0n;
    switch (bytesUnit) {
      case "KB":
        return BigInt(value) * 1024n;
      case "MB":
        return BigInt(value) * 1024n * 1024n;
      default:
        return BigInt(value);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const txCount = BigInt(parseInt(transactions, 10) || 0);
    const bytesValue = getBytesValue();
    await onSubmit(address, txCount, bytesValue);
  };

  const canSubmit = address.length > 0 && (parseInt(transactions, 10) > 0 || getBytesValue() > 0n);

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <div className="space-y-2">
        <label className="text-sm font-medium">Account Address</label>
        <Input
          placeholder="Enter SS58 address..."
          value={address}
          onChange={(e) => setAddress(e.target.value)}
          className="font-mono"
          disabled={isSubmitting}
        />
      </div>
      <div className="space-y-2">
        <label className="text-sm font-medium">Transactions</label>
        <Input
          type="number"
          placeholder="Number of transactions"
          value={transactions}
          onChange={(e) => setTransactions(e.target.value)}
          min="0"
          disabled={isSubmitting}
        />
      </div>
      <div className="space-y-2">
        <label className="text-sm font-medium">Bytes</label>
        <div className="flex gap-2">
          <Input
            type="number"
            placeholder="Amount"
            value={bytes}
            onChange={(e) => setBytes(e.target.value)}
            min="0"
            className="flex-1"
            disabled={isSubmitting}
          />
          <select
            value={bytesUnit}
            onChange={(e) => setBytesUnit(e.target.value as "B" | "KB" | "MB")}
            className="px-3 py-2 border rounded-md bg-background text-sm"
            disabled={isSubmitting}
          >
            <option value="B">Bytes</option>
            <option value="KB">KB</option>
            <option value="MB">MB</option>
          </select>
        </div>
        {bytes && (
          <p className="text-xs text-muted-foreground">
            = {formatBytes(getBytesValue())}
          </p>
        )}
      </div>
      <DialogFooter>
        <Button type="submit" disabled={!canSubmit || isSubmitting}>
          {isSubmitting ? (
            <>
              <Spinner size="sm" className="mr-2" />
              Submitting...
            </>
          ) : (
            "Authorize Account"
          )}
        </Button>
      </DialogFooter>
    </form>
  );
}

interface AuthorizePreimageFormProps {
  onSubmit: (contentHash: string, maxSize: bigint) => Promise<void>;
  isSubmitting: boolean;
}

function AuthorizePreimageForm({ onSubmit, isSubmitting }: AuthorizePreimageFormProps) {
  const [contentHash, setContentHash] = useState("");
  const [maxSize, setMaxSize] = useState("");
  const [sizeUnit, setSizeUnit] = useState<"B" | "KB" | "MB">("KB");

  const getSizeValue = (): bigint => {
    const value = parseInt(maxSize, 10);
    if (isNaN(value)) return 0n;
    switch (sizeUnit) {
      case "KB":
        return BigInt(value) * 1024n;
      case "MB":
        return BigInt(value) * 1024n * 1024n;
      default:
        return BigInt(value);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    await onSubmit(contentHash, getSizeValue());
  };

  const isValidHex = contentHash.length === 0 || /^(0x)?[0-9a-fA-F]{64}$/.test(contentHash);
  const canSubmit = /^(0x)?[0-9a-fA-F]{64}$/.test(contentHash) && getSizeValue() > 0n;

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <div className="space-y-2">
        <label className="text-sm font-medium">Content Hash</label>
        <Input
          placeholder="0x... (32 bytes hex)"
          value={contentHash}
          onChange={(e) => setContentHash(e.target.value)}
          className="font-mono"
          disabled={isSubmitting}
        />
        {!isValidHex && contentHash.length > 0 && (
          <p className="text-xs text-destructive">Must be a 32-byte hex string</p>
        )}
      </div>
      <div className="space-y-2">
        <label className="text-sm font-medium">Max Size</label>
        <div className="flex gap-2">
          <Input
            type="number"
            placeholder="Maximum size"
            value={maxSize}
            onChange={(e) => setMaxSize(e.target.value)}
            min="1"
            className="flex-1"
            disabled={isSubmitting}
          />
          <select
            value={sizeUnit}
            onChange={(e) => setSizeUnit(e.target.value as "B" | "KB" | "MB")}
            className="px-3 py-2 border rounded-md bg-background text-sm"
            disabled={isSubmitting}
          >
            <option value="B">Bytes</option>
            <option value="KB">KB</option>
            <option value="MB">MB</option>
          </select>
        </div>
        {maxSize && (
          <p className="text-xs text-muted-foreground">
            = {formatBytes(getSizeValue())}
          </p>
        )}
      </div>
      <DialogFooter>
        <Button type="submit" disabled={!canSubmit || isSubmitting}>
          {isSubmitting ? (
            <>
              <Spinner size="sm" className="mr-2" />
              Submitting...
            </>
          ) : (
            "Authorize Preimage"
          )}
        </Button>
      </DialogFooter>
    </form>
  );
}

function AccountAuthorizationsTab() {
  const api = useApi();
  const selectedAccount = useSelectedAccount();
  const sudoKey = useSudoKey();
  const authorization = useAuthorization();
  const isLoading = useAuthorizationLoading();

  const [searchAddress, setSearchAddress] = useState("");
  const [searchResult, setSearchResult] = useState<{
    address: string;
    authorization: { transactions: bigint; bytes: bigint; expiresAt?: number } | null;
  } | null>(null);
  const [isSearching, setIsSearching] = useState(false);
  const [isAuthorizeDialogOpen, setIsAuthorizeDialogOpen] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitSuccess, setSubmitSuccess] = useState<string | null>(null);

  const isSudo = selectedAccount && sudoKey && selectedAccount.address === sudoKey;

  const handleAuthorizeAccount = async (address: string, transactions: bigint, bytes: bigint) => {
    if (!api || !selectedAccount) return;

    setIsSubmitting(true);
    setSubmitError(null);
    setSubmitSuccess(null);

    try {
      const authCall = api.tx.TransactionStorage.authorize_account({
        who: address as SS58String,
        transactions: Number(transactions),
        bytes,
      });

      const sudoTx = api.tx.Sudo.sudo({ call: authCall.decodedCall });

      await new Promise<void>((resolve, reject) => {
        let resolved = false;

        const subscription = sudoTx.signSubmitAndWatch(selectedAccount.polkadotSigner).subscribe({
          next: (ev: any) => {
            console.log("TX event:", ev.type);
            if (ev.type === "txBestBlocksState" && ev.found && !resolved) {
              resolved = true;
              subscription.unsubscribe();
              resolve();
            }
          },
          error: (err: any) => {
            if (!resolved) {
              resolved = true;
              reject(err);
            }
          },
        });

        setTimeout(() => {
          if (!resolved) {
            resolved = true;
            subscription.unsubscribe();
            reject(new Error("Transaction timed out"));
          }
        }, 120000);
      });

      setSubmitSuccess(`Successfully authorized account ${formatAddress(address, 8)}`);
      setIsAuthorizeDialogOpen(false);

      // Refresh the authorization if it was for the current account
      if (address === selectedAccount.address) {
        fetchAccountAuthorization(api, selectedAccount.address as SS58String);
      }
    } catch (err) {
      console.error("Authorization failed:", err);
      setSubmitError(err instanceof Error ? err.message : "Authorization failed");
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleSearch = async () => {
    if (!api || !searchAddress) return;

    setIsSearching(true);
    try {
      const auth = await api.query.TransactionStorage.Authorizations.getValue(
        Enum("Account", searchAddress)
      );

      setSearchResult({
        address: searchAddress,
        authorization: auth
          ? {
              transactions: BigInt(auth.extent.transactions),
              bytes: auth.extent.bytes,
              expiresAt: auth.expiration ?? undefined,
            }
          : null,
      });
    } catch (err) {
      console.error("Search failed:", err);
      setSearchResult({ address: searchAddress, authorization: null });
    } finally {
      setIsSearching(false);
    }
  };

  const handleRefresh = () => {
    if (api && selectedAccount) {
      fetchAccountAuthorization(api, selectedAccount.address as SS58String);
    }
  };

  return (
    <div className="space-y-6">
      {/* Success/Error Messages */}
      {submitSuccess && (
        <div className="p-4 rounded-md bg-green-500/10 border border-green-500/20 text-green-600 dark:text-green-400">
          {submitSuccess}
        </div>
      )}
      {submitError && (
        <div className="p-4 rounded-md bg-destructive/10 border border-destructive/20 text-destructive">
          {submitError}
        </div>
      )}

      {/* Sudo Authorization Card */}
      {isSudo && (
        <Card className="border-amber-500/50 bg-amber-500/5">
          <CardHeader>
            <div className="flex items-center justify-between">
              <div>
                <CardTitle className="flex items-center gap-2">
                  <Shield className="h-5 w-5 text-amber-500" />
                  Sudo Access
                </CardTitle>
                <CardDescription>
                  You have sudo privileges. You can authorize accounts to use storage.
                </CardDescription>
              </div>
              <Dialog open={isAuthorizeDialogOpen} onOpenChange={setIsAuthorizeDialogOpen}>
                <DialogTrigger asChild>
                  <Button>
                    <Plus className="h-4 w-4 mr-2" />
                    Authorize Account
                  </Button>
                </DialogTrigger>
                <DialogContent>
                  <DialogHeader>
                    <DialogTitle>Authorize Account</DialogTitle>
                    <DialogDescription>
                      Grant storage authorization to an account. This requires sudo privileges.
                    </DialogDescription>
                  </DialogHeader>
                  <AuthorizeAccountForm
                    onSubmit={handleAuthorizeAccount}
                    isSubmitting={isSubmitting}
                  />
                </DialogContent>
              </Dialog>
            </div>
          </CardHeader>
        </Card>
      )}

      {/* Current Account Authorization */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="flex items-center gap-2">
                <User className="h-5 w-5" />
                Your Authorization
              </CardTitle>
              <CardDescription>
                {selectedAccount
                  ? formatAddress(selectedAccount.address, 8)
                  : "Connect a wallet to view"}
              </CardDescription>
            </div>
            <Button
              variant="ghost"
              size="icon"
              onClick={handleRefresh}
              disabled={isLoading || !api || !selectedAccount}
            >
              <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          {!selectedAccount ? (
            <div className="text-center text-muted-foreground py-4">
              <User className="h-8 w-8 mx-auto mb-2" />
              <p>Connect a wallet to view your authorization</p>
            </div>
          ) : isLoading ? (
            <div className="flex justify-center py-4">
              <Spinner />
            </div>
          ) : authorization ? (
            <div className="grid sm:grid-cols-3 gap-4">
              <div className="space-y-1">
                <p className="text-xs text-muted-foreground uppercase tracking-wide">
                  Transactions Remaining
                </p>
                <p className="text-2xl font-semibold">
                  {authorization.transactions.toLocaleString()}
                </p>
              </div>
              <div className="space-y-1">
                <p className="text-xs text-muted-foreground uppercase tracking-wide">
                  Bytes Remaining
                </p>
                <p className="text-2xl font-semibold">
                  {formatBytes(authorization.bytes)}
                </p>
              </div>
              {authorization.expiresAt && (
                <div className="space-y-1">
                  <p className="text-xs text-muted-foreground uppercase tracking-wide">
                    Expires at Block
                  </p>
                  <p className="text-2xl font-semibold">
                    #{authorization.expiresAt.toLocaleString()}
                  </p>
                </div>
              )}
            </div>
          ) : (
            <div className="text-center text-muted-foreground py-4">
              <AlertCircle className="h-8 w-8 mx-auto mb-2" />
              <p>No authorization found for this account</p>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Search Other Accounts */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Search className="h-5 w-5" />
            Lookup Account
          </CardTitle>
          <CardDescription>
            Check authorization for any account
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex gap-2">
            <Input
              placeholder="Enter SS58 address..."
              value={searchAddress}
              onChange={(e) => setSearchAddress(e.target.value)}
              className="font-mono"
            />
            <Button
              onClick={handleSearch}
              disabled={!api || !searchAddress || isSearching}
            >
              {isSearching ? <Spinner size="sm" /> : "Search"}
            </Button>
          </div>

          {searchResult && (
            <div className="p-4 rounded-md bg-secondary/50 border">
              <p className="font-mono text-sm mb-3 truncate">
                {searchResult.address}
              </p>
              {searchResult.authorization ? (
                <div className="grid sm:grid-cols-2 gap-3 text-sm">
                  <div>
                    <span className="text-muted-foreground">Transactions:</span>{" "}
                    <span className="font-medium">
                      {searchResult.authorization.transactions.toLocaleString()}
                    </span>
                  </div>
                  <div>
                    <span className="text-muted-foreground">Bytes:</span>{" "}
                    <span className="font-medium">
                      {formatBytes(searchResult.authorization.bytes)}
                    </span>
                  </div>
                </div>
              ) : (
                <p className="text-muted-foreground">No authorization found</p>
              )}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function PreimageAuthorizationsTab() {
  const api = useApi();
  const selectedAccount = useSelectedAccount();
  const sudoKey = useSudoKey();
  const preimageAuths = usePreimageAuthorizations();
  const isLoading = usePreimageAuthsLoading();

  const [isAuthorizeDialogOpen, setIsAuthorizeDialogOpen] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitSuccess, setSubmitSuccess] = useState<string | null>(null);

  const isSudo = selectedAccount && sudoKey && selectedAccount.address === sudoKey;

  const handleAuthorizePreimage = async (contentHash: string, maxSize: bigint) => {
    if (!api || !selectedAccount) return;

    setIsSubmitting(true);
    setSubmitError(null);
    setSubmitSuccess(null);

    try {
      // Normalize the content hash (ensure it has 0x prefix)
      const normalizedHash = contentHash.startsWith("0x") ? contentHash : `0x${contentHash}`;

      const authCall = api.tx.TransactionStorage.authorize_preimage({
        content_hash: Binary.fromHex(normalizedHash),
        max_size: maxSize,
      });

      const sudoTx = api.tx.Sudo.sudo({ call: authCall.decodedCall });

      await new Promise<void>((resolve, reject) => {
        let resolved = false;

        const subscription = sudoTx.signSubmitAndWatch(selectedAccount.polkadotSigner).subscribe({
          next: (ev: any) => {
            console.log("TX event:", ev.type);
            if (ev.type === "txBestBlocksState" && ev.found && !resolved) {
              resolved = true;
              subscription.unsubscribe();
              resolve();
            }
          },
          error: (err: any) => {
            if (!resolved) {
              resolved = true;
              reject(err);
            }
          },
        });

        setTimeout(() => {
          if (!resolved) {
            resolved = true;
            subscription.unsubscribe();
            reject(new Error("Transaction timed out"));
          }
        }, 120000);
      });

      setSubmitSuccess(`Successfully authorized preimage`);
      setIsAuthorizeDialogOpen(false);

      // Refresh the preimage authorizations list
      fetchPreimageAuthorizations(api);
    } catch (err) {
      console.error("Preimage authorization failed:", err);
      setSubmitError(err instanceof Error ? err.message : "Authorization failed");
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleRefresh = () => {
    if (api) {
      fetchPreimageAuthorizations(api);
    }
  };

  useEffect(() => {
    if (api && preimageAuths.length === 0) {
      fetchPreimageAuthorizations(api);
    }
  }, [api, preimageAuths.length]);

  return (
    <div className="space-y-6">
      {/* Success/Error Messages */}
      {submitSuccess && (
        <div className="p-4 rounded-md bg-green-500/10 border border-green-500/20 text-green-600 dark:text-green-400">
          {submitSuccess}
        </div>
      )}
      {submitError && (
        <div className="p-4 rounded-md bg-destructive/10 border border-destructive/20 text-destructive">
          {submitError}
        </div>
      )}

      {/* Sudo Authorization Card */}
      {isSudo && (
        <Card className="border-amber-500/50 bg-amber-500/5">
          <CardHeader>
            <div className="flex items-center justify-between">
              <div>
                <CardTitle className="flex items-center gap-2">
                  <Shield className="h-5 w-5 text-amber-500" />
                  Sudo Access
                </CardTitle>
                <CardDescription>
                  You have sudo privileges. You can authorize preimages for unsigned uploads.
                </CardDescription>
              </div>
              <Dialog open={isAuthorizeDialogOpen} onOpenChange={setIsAuthorizeDialogOpen}>
                <DialogTrigger asChild>
                  <Button>
                    <Plus className="h-4 w-4 mr-2" />
                    Authorize Preimage
                  </Button>
                </DialogTrigger>
                <DialogContent>
                  <DialogHeader>
                    <DialogTitle>Authorize Preimage</DialogTitle>
                    <DialogDescription>
                      Authorize a content hash for unsigned uploads. This requires sudo privileges.
                    </DialogDescription>
                  </DialogHeader>
                  <AuthorizePreimageForm
                    onSubmit={handleAuthorizePreimage}
                    isSubmitting={isSubmitting}
                  />
                </DialogContent>
              </Dialog>
            </div>
          </CardHeader>
        </Card>
      )}

      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="flex items-center gap-2">
                <FileText className="h-5 w-5" />
                Preimage Authorizations
              </CardTitle>
              <CardDescription>
                Content hashes authorized for unsigned uploads
              </CardDescription>
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
          <div className="flex justify-center py-8">
            <Spinner />
          </div>
        ) : preimageAuths.length === 0 ? (
          <div className="text-center text-muted-foreground py-8">
            <FileText className="h-8 w-8 mx-auto mb-2" />
            <p>No preimage authorizations found</p>
          </div>
        ) : (
          <div className="space-y-3">
            {preimageAuths.map((auth, index) => (
              <div
                key={index}
                className="p-4 rounded-md bg-secondary/50 border"
              >
                <div className="flex items-start justify-between gap-4">
                  <div className="flex-1 min-w-0">
                    <p className="text-xs text-muted-foreground mb-1">Content Hash</p>
                    <p className="font-mono text-sm truncate">
                      {bytesToHex(auth.contentHash)}
                    </p>
                  </div>
                  <Badge variant="secondary">
                    Max {formatBytes(auth.maxSize)}
                  </Badge>
                </div>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
    </div>
  );
}

function StorageFaucetTab() {
  const api = useApi();
  const sudoKey = useSudoKey();

  const [forWho, setForWho] = useState("");
  const [transactions, setTransactions] = useState("100");
  const [bytes, setBytes] = useState("10");
  const [bytesUnit, setBytesUnit] = useState<"B" | "KB" | "MB">("MB");

  const [authorization, setAuthorization] = useState<{
    transactions: bigint;
    bytes: bigint;
    expiresAt?: number;
  } | null>(null);
  const [aliceAddress, setAliceAddress] = useState<string>("");
  const [aliceBalance, setAliceBalance] = useState<bigint | null>(null);
  const [isLoadingAuth, setIsLoadingAuth] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitSuccess, setSubmitSuccess] = useState<string | null>(null);

  // Initialize Alice account and check balance
  useEffect(() => {
    const initAlice = async () => {
      if (!api) return;

      try {
        await cryptoWaitReady();
        const keyring = new Keyring({ type: "sr25519" });
        const alice = keyring.addFromUri("//Alice");
        setAliceAddress(alice.address);

        // Check Alice's balance
        const accountInfo = await api.query.System.Account.getValue(alice.address as SS58String);
        setAliceBalance(accountInfo.data.free);
      } catch (err) {
        console.error("Failed to initialize Alice account:", err);
      }
    };

    initAlice();
  }, [api]);

  const getBytesValue = (): bigint => {
    const value = parseInt(bytes, 10);
    if (isNaN(value)) return 0n;
    switch (bytesUnit) {
      case "KB":
        return BigInt(value) * 1024n;
      case "MB":
        return BigInt(value) * 1024n * 1024n;
      default:
        return BigInt(value);
    }
  };

  // Auto-fetch authorization when forWho address changes
  useEffect(() => {
    if (!api || !forWho) {
      setAuthorization(null);
      return;
    }

    // Validate SS58 address format
    if (forWho.length < 40) {
      return;
    }

    const fetchAuth = async () => {
      setIsLoadingAuth(true);
      try {
        const auth = await api.query.TransactionStorage.Authorizations.getValue(
          Enum("Account", forWho)
        );

        setAuthorization(
          auth
            ? {
                transactions: BigInt(auth.extent.transactions),
                bytes: auth.extent.bytes,
                expiresAt: auth.expiration ?? undefined,
              }
            : null
        );
      } catch (err) {
        console.error("Failed to fetch authorization:", err);
        setAuthorization(null);
      } finally {
        setIsLoadingAuth(false);
      }
    };

    fetchAuth();
  }, [api, forWho]);

  const handleAuthorize = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!api || !forWho) return;

    setIsSubmitting(true);
    setSubmitError(null);
    setSubmitSuccess(null);

    try {
      // Ensure crypto is ready
      await cryptoWaitReady();

      // Create Alice dev account signer using Polkadot.js keyring
      const keyring = new Keyring({ type: "sr25519" });
      const alice = keyring.addFromUri("//Alice");

      // Create polkadot-api compatible signer
      const aliceSigner = getPolkadotSigner(
        alice.publicKey,
        "Sr25519",
        (data: Uint8Array) => alice.sign(data)
      );

      const txCount = BigInt(parseInt(transactions, 10) || 0);
      const bytesValue = getBytesValue();

      const authCall = api.tx.TransactionStorage.authorize_account({
        who: forWho as SS58String,
        transactions: Number(txCount),
        bytes: bytesValue,
      });

      const sudoTx = api.tx.Sudo.sudo({ call: authCall.decodedCall });

      await new Promise<void>((resolve, reject) => {
        let resolved = false;

        const subscription = sudoTx.signSubmitAndWatch(aliceSigner).subscribe({
          next: (ev: any) => {
            console.log("TX event:", ev.type);
            if (ev.type === "txBestBlocksState" && ev.found && !resolved) {
              resolved = true;
              subscription.unsubscribe();
              resolve();
            }
          },
          error: (err: any) => {
            if (!resolved) {
              resolved = true;
              reject(err);
            }
          },
        });

        setTimeout(() => {
          if (!resolved) {
            resolved = true;
            subscription.unsubscribe();
            reject(new Error("Transaction timed out"));
          }
        }, 120000);
      });

      setSubmitSuccess(`Successfully authorized account ${formatAddress(forWho, 8)}`);

      // Refresh authorization display
      const auth = await api.query.TransactionStorage.Authorizations.getValue(
        Enum("Account", forWho)
      );
      setAuthorization(
        auth
          ? {
              transactions: BigInt(auth.extent.transactions),
              bytes: auth.extent.bytes,
              expiresAt: auth.expiration ?? undefined,
            }
          : null
      );
    } catch (err) {
      console.error("Authorization failed:", err);

      let errorMessage = "Authorization failed";
      if (err instanceof Error) {
        errorMessage = err.message;
      } else if (typeof err === "object" && err !== null) {
        const errObj = err as any;
        if (errObj.type === "Invalid" && errObj.value?.type === "Payment") {
          errorMessage = "Payment error: Alice account has insufficient balance to pay transaction fees. Please fund Alice's account or use a local dev chain where Alice has initial funds.";
        } else {
          errorMessage = JSON.stringify(err);
        }
      }

      setSubmitError(errorMessage);
    } finally {
      setIsSubmitting(false);
    }
  };

  const hasBalanceIssue = aliceBalance !== null && aliceBalance === 0n;
  const hasSudoIssue = aliceAddress && sudoKey && aliceAddress !== sudoKey;
  const canSubmit =
    forWho.length > 0 &&
    (parseInt(transactions, 10) > 0 || getBytesValue() > 0n) &&
    !isSubmitting &&
    !hasBalanceIssue &&
    !hasSudoIssue;

  return (
    <div className="space-y-6">
      {/* Success/Error Messages */}
      {submitSuccess && (
        <div className="p-4 rounded-md bg-green-500/10 border border-green-500/20 text-green-600 dark:text-green-400">
          {submitSuccess}
        </div>
      )}
      {submitError && (
        <div className="p-4 rounded-md bg-destructive/10 border border-destructive/20 text-destructive">
          {submitError}
        </div>
      )}

      {/* Info Card */}
      <Card className="border-blue-500/50 bg-blue-500/5">
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Droplet className="h-5 w-5 text-blue-500" />
            Storage Faucet
          </CardTitle>
          <CardDescription>
            Authorize storage allowances for accounts using the Alice dev account. This is for testing purposes only.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="space-y-2 text-sm">
            {aliceAddress && (
              <div className="flex items-center justify-between p-3 rounded-md bg-secondary/50">
                <span className="text-muted-foreground">Alice Address:</span>
                <span className="font-mono">{formatAddress(aliceAddress, 8)}</span>
              </div>
            )}
            {aliceBalance !== null && (
              <div className="flex items-center justify-between p-3 rounded-md bg-secondary/50">
                <span className="text-muted-foreground">Alice Balance:</span>
                <span className="font-mono">{formatBytes(aliceBalance)}</span>
              </div>
            )}
            {aliceBalance !== null && aliceBalance === 0n && (
              <div className="p-3 rounded-md bg-amber-500/10 border border-amber-500/20 text-amber-600 dark:text-amber-400">
                <AlertCircle className="h-4 w-4 inline mr-2" />
                Warning: Alice account has zero balance. Transactions will fail. Please fund Alice or use a local dev chain.
              </div>
            )}
            {aliceAddress && sudoKey && aliceAddress !== sudoKey && (
              <div className="p-3 rounded-md bg-amber-500/10 border border-amber-500/20 text-amber-600 dark:text-amber-400">
                <AlertCircle className="h-4 w-4 inline mr-2" />
                Warning: Alice is not the sudo key on this network. Sudo calls will fail.
              </div>
            )}
          </div>
        </CardContent>
      </Card>

      {/* Authorization Form */}
      <Card>
        <CardHeader>
          <CardTitle>Authorize Account</CardTitle>
          <CardDescription>
            Grant storage authorization to any account. Signed automatically with //Alice.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleAuthorize} className="space-y-4">
            <div className="space-y-2">
              <label className="text-sm font-medium">
                Account Address (for_who)
              </label>
              <Input
                placeholder="Enter SS58 address..."
                value={forWho}
                onChange={(e) => setForWho(e.target.value)}
                className="font-mono"
                disabled={isSubmitting}
              />

              {/* Auto-display current authorization */}
              {forWho && forWho.length >= 40 && (
                <div className="mt-3 p-3 rounded-md bg-secondary/50 border">
                  <p className="text-xs text-muted-foreground mb-2">Current Authorization:</p>
                  {isLoadingAuth ? (
                    <div className="flex items-center gap-2">
                      <Spinner size="sm" />
                      <span className="text-sm text-muted-foreground">Loading...</span>
                    </div>
                  ) : authorization ? (
                    <div className="grid sm:grid-cols-2 gap-2 text-sm">
                      <div>
                        <span className="text-muted-foreground">Transactions:</span>{" "}
                        <span className="font-medium">
                          {authorization.transactions.toLocaleString()}
                        </span>
                      </div>
                      <div>
                        <span className="text-muted-foreground">Bytes:</span>{" "}
                        <span className="font-medium">
                          {formatBytes(authorization.bytes)}
                        </span>
                      </div>
                      {authorization.expiresAt && (
                        <div className="sm:col-span-2">
                          <span className="text-muted-foreground">Expires at block:</span>{" "}
                          <span className="font-medium">
                            #{authorization.expiresAt.toLocaleString()}
                          </span>
                        </div>
                      )}
                    </div>
                  ) : (
                    <p className="text-sm text-muted-foreground">No authorization found</p>
                  )}
                </div>
              )}
            </div>

            <div className="space-y-2">
              <label className="text-sm font-medium">Transactions</label>
              <Input
                type="number"
                placeholder="Number of transactions"
                value={transactions}
                onChange={(e) => setTransactions(e.target.value)}
                min="0"
                disabled={isSubmitting}
              />
            </div>

            <div className="space-y-2">
              <label className="text-sm font-medium">Bytes</label>
              <div className="flex gap-2">
                <Input
                  type="number"
                  placeholder="Amount"
                  value={bytes}
                  onChange={(e) => setBytes(e.target.value)}
                  min="0"
                  className="flex-1"
                  disabled={isSubmitting}
                />
                <select
                  value={bytesUnit}
                  onChange={(e) => setBytesUnit(e.target.value as "B" | "KB" | "MB")}
                  className="px-3 py-2 border rounded-md bg-background text-sm"
                  disabled={isSubmitting}
                >
                  <option value="B">Bytes</option>
                  <option value="KB">KB</option>
                  <option value="MB">MB</option>
                </select>
              </div>
              {bytes && (
                <p className="text-xs text-muted-foreground">
                  = {formatBytes(getBytesValue())}
                </p>
              )}
            </div>

            <Button type="submit" disabled={!canSubmit} className="w-full">
              {isSubmitting ? (
                <>
                  <Spinner size="sm" className="mr-2" />
                  Authorizing...
                </>
              ) : hasBalanceIssue ? (
                <>
                  <AlertCircle className="h-4 w-4 mr-2" />
                  Alice Has No Balance
                </>
              ) : hasSudoIssue ? (
                <>
                  <AlertCircle className="h-4 w-4 mr-2" />
                  Alice Is Not Sudo
                </>
              ) : (
                <>
                  <Droplet className="h-4 w-4 mr-2" />
                  Authorize User
                </>
              )}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}

export function Authorizations() {
  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Authorizations</h1>
        <p className="text-muted-foreground">
          View and manage storage authorizations
        </p>
      </div>

      <Tabs defaultValue="accounts">
        <TabsList>
          <TabsTrigger value="accounts">
            <User className="h-4 w-4 mr-2" />
            Account
          </TabsTrigger>
          <TabsTrigger value="preimages">
            <FileText className="h-4 w-4 mr-2" />
            Preimages
          </TabsTrigger>
          <TabsTrigger value="faucet">
            <Droplet className="h-4 w-4 mr-2" />
            Storage Faucet
          </TabsTrigger>
        </TabsList>
        <TabsContent value="accounts" className="mt-4">
          <AccountAuthorizationsTab />
        </TabsContent>
        <TabsContent value="preimages" className="mt-4">
          <PreimageAuthorizationsTab />
        </TabsContent>
        <TabsContent value="faucet" className="mt-4">
          <StorageFaucetTab />
        </TabsContent>
      </Tabs>
    </div>
  );
}
