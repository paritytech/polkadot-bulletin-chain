import { useState, useEffect, useCallback } from "react";
import { useSearchParams } from "react-router-dom";
import { RefreshCw, User, FileText, AlertCircle, Search, Droplet } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Textarea } from "@/components/ui/Textarea";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/Tabs";
import { useApi, useCreateBulletinClient } from "@/state/chain.state";
import { useSelectedAccount } from "@/state/wallet.state";
import {
  useAuthorization,
  useAuthorizationLoading,
  usePreimageAuthorizations,
  usePreimageAuthsLoading,
  fetchAccountAuthorization,
  fetchPreimageAuthorizations,
} from "@/state/storage.state";
import { FileUpload } from "@/components/FileUpload";
import { getContentHash, HashAlgorithm, ProgressEvent } from "@bulletin/sdk";
import { bytesToHex, hexToBytes } from "@/utils/format";
import { formatBytes, formatNumber, formatAddress } from "@/utils/format";
import { SS58String, Enum } from "polkadot-api";
import { cryptoWaitReady } from "@polkadot/util-crypto";
import { Keyring } from "@polkadot/keyring";
import { getPolkadotSigner } from "polkadot-api/signer";

function AccountAuthorizationsTab() {
  const api = useApi();
  const selectedAccount = useSelectedAccount();
  const authorization = useAuthorization();
  const isLoading = useAuthorizationLoading();

  const [searchAddress, setSearchAddress] = useState("");
  const [searchResult, setSearchResult] = useState<{
    address: string;
    authorization: { transactions: bigint; bytes: bigint; expiresAt?: number } | null;
  } | null>(null);
  const [isSearching, setIsSearching] = useState(false);

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
                  {formatNumber(Number(authorization.transactions))}
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
                    #{formatNumber(authorization.expiresAt)}
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
                <div className="grid grid-cols-3 gap-4">
                  <div className="space-y-1">
                    <p className="text-xs text-muted-foreground uppercase tracking-wide">
                      Transactions
                    </p>
                    <p className="text-2xl font-semibold">
                      {formatNumber(Number(searchResult.authorization.transactions))}
                    </p>
                  </div>
                  <div className="space-y-1">
                    <p className="text-xs text-muted-foreground uppercase tracking-wide">
                      Bytes
                    </p>
                    <p className="text-2xl font-semibold">
                      {formatBytes(searchResult.authorization.bytes)}
                    </p>
                  </div>
                  <div className="space-y-1">
                    <p className="text-xs text-muted-foreground uppercase tracking-wide">
                      Expires at Block
                    </p>
                    <p className="text-2xl font-semibold">
                      {searchResult.authorization.expiresAt
                        ? `#${formatNumber(searchResult.authorization.expiresAt)}`
                        : "Never"}
                    </p>
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
  const preimageAuths = usePreimageAuthorizations();
  const isLoading = usePreimageAuthsLoading();

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

function FaucetAuthorizePreimagePanel() {
  const api = useApi();
  const createBulletinClient = useCreateBulletinClient();

  const [preimageHash, setPreimageHash] = useState("");
  const [inputMode, setInputMode] = useState<"text" | "file">("text");
  const [textData, setTextData] = useState("");
  const [fileData, setFileData] = useState<Uint8Array | null>(null);
  const [fileName, setFileName] = useState<string | null>(null);
  const [maxSize, setMaxSize] = useState("");
  const [sizeUnit, setSizeUnit] = useState<"B" | "KB" | "MB">("KB");
  const [isComputing, setIsComputing] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitSuccess, setSubmitSuccess] = useState<string | null>(null);
  const [txStatus, setTxStatus] = useState<string | null>(null);

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

  // Compute blake2 hash when text changes
  useEffect(() => {
    if (inputMode !== "text" || !textData.trim()) return;

    const computeHash = async () => {
      setIsComputing(true);
      try {
        const data = new TextEncoder().encode(textData);
        const hash = await getContentHash(data, HashAlgorithm.Blake2b256);
        setPreimageHash(bytesToHex(hash));
        setMaxSize(data.length.toString());
        setSizeUnit("B");
      } catch (err) {
        console.error("Failed to compute hash:", err);
      } finally {
        setIsComputing(false);
      }
    };

    computeHash();
  }, [textData, inputMode]);

  // Compute blake2 hash when file changes
  useEffect(() => {
    if (inputMode !== "file" || !fileData) return;

    const computeHash = async () => {
      setIsComputing(true);
      try {
        const hash = await getContentHash(fileData, HashAlgorithm.Blake2b256);
        setPreimageHash(bytesToHex(hash));
        setMaxSize(fileData.length.toString());
        setSizeUnit("B");
      } catch (err) {
        console.error("Failed to compute hash:", err);
      } finally {
        setIsComputing(false);
      }
    };

    computeHash();
  }, [fileData, inputMode]);

  const handleFileSelect = useCallback((file: File | null, data: Uint8Array | null) => {
    setFileData(data);
    setFileName(file?.name ?? null);
    if (!data) {
      setPreimageHash("");
      setMaxSize("");
    }
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!api || !preimageHash) return;

    setIsSubmitting(true);
    setSubmitError(null);
    setSubmitSuccess(null);
    setTxStatus(null);

    try {
      await cryptoWaitReady();
      const keyring = new Keyring({ type: "sr25519" });
      const alice = keyring.addFromUri("//Alice");
      const aliceSigner = getPolkadotSigner(
        alice.publicKey,
        "Sr25519",
        (data: Uint8Array) => alice.sign(data)
      );

      const normalizedHash = preimageHash.startsWith("0x") ? preimageHash.slice(2) : preimageHash;
      const contentHashBytes = hexToBytes(normalizedHash);
      const sizeValue = getSizeValue();

<<<<<<< naren-client
      // Create SDK client with Alice signer
      const bulletinClient = createBulletinClient!(aliceSigner);

      // Progress callback for transaction status updates
      const handleProgress = (event: ProgressEvent) => {
        console.log("SDK progress:", event);
        if (event.type === "signed") {
          setTxStatus("Transaction signed...");
        } else if (event.type === "broadcasted") {
          setTxStatus("Broadcasting to network...");
        } else if (event.type === "best_block") {
          setTxStatus(`Included in block #${event.blockNumber}...`);
        } else if (event.type === "finalized") {
          setTxStatus("Finalized!");
        }
      };

      // Use SDK to authorize preimage with progress callback
      await bulletinClient.authorizePreimage(
        contentHashBytes,
        sizeValue > 0n ? sizeValue : 1024n * 1024n,
        handleProgress
      );
=======
      const tx = api.tx.TransactionStorage.authorize_preimage({
        content_hash: Binary.fromHex(normalizedHash),
        max_size: sizeValue > 0n ? sizeValue : 1024n * 1024n,
      });

      // Wait for finalization (not just best block inclusion) so that
      // subsequent storage queries can see the new authorization.
      const result = await tx.signAndSubmit(aliceSigner);
      if (!result.ok) {
        throw new Error("Transaction dispatch failed");
      }
>>>>>>> main

      setSubmitSuccess("Successfully authorized preimage");
      fetchPreimageAuthorizations(api);
    } catch (err) {
      console.error("Preimage authorization failed:", err);
      setSubmitError(err instanceof Error ? err.message : "Authorization failed");
    } finally {
      setIsSubmitting(false);
      setTxStatus(null);
    }
  };

  const isValidHash = preimageHash.length === 0 || /^(0x)?[0-9a-fA-F]{64}$/.test(preimageHash);
  const canSubmit = /^(0x)?[0-9a-fA-F]{64}$/.test(preimageHash) && !isSubmitting;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <FileText className="h-5 w-5" />
          Authorize Preimage
        </CardTitle>
        <CardDescription>
          Authorize a content hash for storage. Compute blake2 hash from text or file, or enter it directly.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {submitSuccess && (
          <div className="mb-4 p-4 rounded-md bg-green-500/10 border border-green-500/20 text-green-600 dark:text-green-400">
            {submitSuccess}
          </div>
        )}
        {submitError && (
          <div className="mb-4 p-4 rounded-md bg-destructive/10 border border-destructive/20 text-destructive">
            {submitError}
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <label className="text-sm font-medium">Blake2 Hash (required)</label>
            <div className="relative">
              <Input
                placeholder="0x... (32 bytes hex)"
                value={preimageHash}
                onChange={(e) => setPreimageHash(e.target.value)}
                className="font-mono"
                disabled={isSubmitting}
              />
              {isComputing && (
                <div className="absolute right-3 top-1/2 -translate-y-1/2">
                  <Spinner size="sm" />
                </div>
              )}
            </div>
            {!isValidHash && preimageHash.length > 0 && (
              <p className="text-xs text-destructive">Must be a 32-byte hex string (64 hex chars)</p>
            )}
          </div>

          <div className="space-y-2">
            <label className="text-sm font-medium text-muted-foreground">
              Compute hash from data (optional)
            </label>
            <Tabs value={inputMode} onValueChange={(v) => setInputMode(v as "text" | "file")}>
              <TabsList>
                <TabsTrigger value="text">Text</TabsTrigger>
                <TabsTrigger value="file">File</TabsTrigger>
              </TabsList>
              <TabsContent value="text" className="space-y-2">
                <Textarea
                  placeholder="Enter text to compute blake2 hash..."
                  value={textData}
                  onChange={(e) => {
                    setTextData(e.target.value);
                    setSubmitSuccess(null);
                    setSubmitError(null);
                  }}
                  className="min-h-[120px] font-mono"
                  disabled={isSubmitting}
                />
              </TabsContent>
              <TabsContent value="file">
                <FileUpload
                  onFileSelect={handleFileSelect}
                  maxSize={10 * 1024 * 1024}
                  disabled={isSubmitting}
                />
                {fileName && (
                  <p className="text-sm text-muted-foreground mt-2">
                    Selected: {fileName}
                  </p>
                )}
              </TabsContent>
            </Tabs>
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

          <Button type="submit" disabled={!canSubmit} className="w-full">
            {isSubmitting ? (
              <>
                <Spinner size="sm" className="mr-2" />
                {txStatus || "Authorizing Preimage..."}
              </>
            ) : (
              <>
                <FileText className="h-4 w-4 mr-2" />
                Authorize Preimage
              </>
            )}
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}

function FaucetAuthorizeAccountPanel() {
  const api = useApi();
  const createBulletinClient = useCreateBulletinClient();
  const selectedAccount = useSelectedAccount();

  const [forWho, setForWho] = useState("");
  const [transactions, setTransactions] = useState("100");
  const [bytes, setBytes] = useState("10");
  const [bytesUnit, setBytesUnit] = useState<"B" | "KB" | "MB">("MB");

  const [authorization, setAuthorization] = useState<{
    transactions: bigint;
    bytes: bigint;
    expiresAt?: number;
  } | null>(null);
  const [aliceBalance, setAliceBalance] = useState<bigint | null>(null);
  const [isLoadingAuth, setIsLoadingAuth] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitSuccess, setSubmitSuccess] = useState<string | null>(null);
  const [txStatus, setTxStatus] = useState<string | null>(null);

  // Initialize Alice account
  useEffect(() => {
    const initAccounts = async () => {
      if (!api) return;

      try {
        await cryptoWaitReady();
        const keyring = new Keyring({ type: "sr25519" });
        const alice = keyring.addFromUri("//Alice");

        const accountInfo = await api.query.System.Account.getValue(alice.address as SS58String);
        setAliceBalance(accountInfo?.data?.free ?? null);
      } catch (err) {
        console.error("Failed to initialize dev accounts:", err);
      }
    };

    initAccounts();
  }, [api]);

  // Prefill with connected account address
  useEffect(() => {
    if (selectedAccount?.address && !forWho) {
      setForWho(selectedAccount.address);
    }
  }, [selectedAccount?.address]);

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
    setTxStatus(null);

    try {
      await cryptoWaitReady();
      const keyring = new Keyring({ type: "sr25519" });
      const alice = keyring.addFromUri("//Alice");
      const aliceSigner = getPolkadotSigner(
        alice.publicKey,
        "Sr25519",
        (data: Uint8Array) => alice.sign(data)
      );

      const txCount = parseInt(transactions, 10) || 0;
      const bytesValue = getBytesValue();

<<<<<<< naren-client
      // Create SDK client with Alice signer
      const bulletinClient = createBulletinClient!(aliceSigner);

      // Progress callback for transaction status updates
      const handleProgress = (event: ProgressEvent) => {
        console.log("SDK progress:", event);
        if (event.type === "signed") {
          setTxStatus("Transaction signed...");
        } else if (event.type === "broadcasted") {
          setTxStatus("Broadcasting to network...");
        } else if (event.type === "best_block") {
          setTxStatus(`Included in block #${event.blockNumber}...`);
        } else if (event.type === "finalized") {
          setTxStatus("Finalized!");
        }
      };

      // Use SDK to authorize account with progress callback
      await bulletinClient.authorizeAccount(
        forWho,
        txCount,
        bytesValue,
        handleProgress
      );
=======
      const tx = api.tx.TransactionStorage.authorize_account({
        who: forWho as SS58String,
        transactions: Number(txCount),
        bytes: bytesValue,
      });

      // Wait for finalization so subsequent queries see the new authorization
      const result = await tx.signAndSubmit(aliceSigner);
      if (!result.ok) {
        throw new Error("Transaction dispatch failed");
      }
>>>>>>> main

      setSubmitSuccess(`Successfully authorized account ${formatAddress(forWho, 8)}`);

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
      setTxStatus(null);
    }
  };

  const hasBalanceIssue = aliceBalance !== null && aliceBalance === 0n;
  const canSubmit =
    forWho.length > 0 &&
    (parseInt(transactions, 10) > 0 || getBytesValue() > 0n) &&
    !isSubmitting &&
    !hasBalanceIssue;

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

      {hasBalanceIssue && (
        <div className="p-3 rounded-md bg-amber-500/10 border border-amber-500/20 text-amber-600 dark:text-amber-400">
          <AlertCircle className="h-4 w-4 inline mr-2" />
          Warning: Alice account has zero balance. Transactions will fail.
        </div>
      )}

      {/* Authorization Form */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <User className="h-5 w-5" />
            Authorize Account
          </CardTitle>
          <CardDescription>
            Grant storage authorization to any account using the Alice dev account.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleAuthorize} className="space-y-4">
            <div className="space-y-2">
              <label className="text-sm font-medium">
                Account Address
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
                          {formatNumber(Number(authorization.transactions))}
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
                            #{formatNumber(authorization.expiresAt)}
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
                  {txStatus || "Authorizing..."}
                </>
              ) : hasBalanceIssue ? (
                <>
                  <AlertCircle className="h-4 w-4 mr-2" />
                  Alice Has No Balance
                </>
              ) : (
                <>
                  <Droplet className="h-4 w-4 mr-2" />
                  Authorize Account
                </>
              )}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}

function StorageFaucetTab() {
  const [faucetTab, setFaucetTab] = useState<"account" | "preimage">("account");

  return (
    <div className="space-y-6">
      {/* Info Card */}
      <Card className="border-blue-500/50 bg-blue-500/5">
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Droplet className="h-5 w-5 text-blue-500" />
            Storage Faucet
          </CardTitle>
          <CardDescription>
            Authorize storage allowances using the Alice dev account. This is for testing purposes only.
          </CardDescription>
        </CardHeader>
      </Card>

      {/* Sub-tabs for Account vs Preimage */}
      <Tabs value={faucetTab} onValueChange={(v) => setFaucetTab(v as "account" | "preimage")}>
        <TabsList className="grid w-full grid-cols-2">
          <TabsTrigger value="account">
            <User className="h-4 w-4 mr-2" />
            Authorize Account
          </TabsTrigger>
          <TabsTrigger value="preimage">
            <FileText className="h-4 w-4 mr-2" />
            Authorize Preimage
          </TabsTrigger>
        </TabsList>
        <TabsContent value="account" className="mt-4">
          <FaucetAuthorizeAccountPanel />
        </TabsContent>
        <TabsContent value="preimage" className="mt-4">
          <FaucetAuthorizePreimagePanel />
        </TabsContent>
      </Tabs>
    </div>
  );
}

export function Authorizations() {
  const [searchParams, setSearchParams] = useSearchParams();
  const activeTab = searchParams.get("tab") || "faucet";

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Faucet & Authorizations</h1>
        <p className="text-muted-foreground">
          Storage faucet and authorization management
        </p>
      </div>

      <Tabs value={activeTab} onValueChange={(v) => setSearchParams({ tab: v })}>
        <TabsList>
          <TabsTrigger value="faucet">
            <Droplet className="h-4 w-4 mr-2" />
            Storage Faucet
          </TabsTrigger>
          <TabsTrigger value="accounts">
            <User className="h-4 w-4 mr-2" />
            Accounts
          </TabsTrigger>
          <TabsTrigger value="preimages">
            <FileText className="h-4 w-4 mr-2" />
            Preimages
          </TabsTrigger>
        </TabsList>
        <TabsContent value="faucet" className="mt-4">
          <StorageFaucetTab />
        </TabsContent>
        <TabsContent value="accounts" className="mt-4">
          <AccountAuthorizationsTab />
        </TabsContent>
        <TabsContent value="preimages" className="mt-4">
          <PreimageAuthorizationsTab />
        </TabsContent>
      </Tabs>
    </div>
  );
}
