import { BehaviorSubject, combineLatest, switchMap, of, from, catchError } from "rxjs";
import { bind } from "@react-rxjs/core";
import { api$ } from "./chain.state";
import { selectedAccount$ } from "./wallet.state";
import { SS58String, Enum, Binary } from "polkadot-api";

export interface Authorization {
  transactions: bigint;
  bytes: bigint;
  expiresAt?: number;
}

export interface PreimageAuthorization {
  contentHash: Uint8Array;
  maxSize: bigint;
}

export interface TransactionInfo {
  chunkRoot: Uint8Array;
  contentHash: Uint8Array;
  size: number;
  blockChunks: number;
}

// Account authorization state
const authorizationSubject = new BehaviorSubject<Authorization | null>(null);
const authorizationLoadingSubject = new BehaviorSubject<boolean>(false);
const authorizationErrorSubject = new BehaviorSubject<string | undefined>(undefined);

export async function fetchAccountAuthorization(
  api: any,
  address: SS58String
): Promise<Authorization | null> {
  authorizationLoadingSubject.next(true);
  authorizationErrorSubject.next(undefined);

  try {
    const auth = await api.query.TransactionStorage.Authorizations.getValue(
      Enum("Account", address)
    );

    if (!auth) {
      authorizationSubject.next(null);
      return null;
    }

    const authorization: Authorization = {
      transactions: BigInt(auth.extent.transactions),
      bytes: auth.extent.bytes,
      expiresAt: auth.expiration ?? undefined,
    };

    authorizationSubject.next(authorization);
    return authorization;
  } catch (err) {
    const message = err instanceof Error ? err.message : "Failed to fetch authorization";
    authorizationErrorSubject.next(message);
    authorizationSubject.next(null);
    return null;
  } finally {
    authorizationLoadingSubject.next(false);
  }
}

// Single preimage authorization check (for Upload page unsigned tx flow)
const preimageAuthSubject = new BehaviorSubject<Authorization | null>(null);
const preimageAuthLoadingSubject = new BehaviorSubject<boolean>(false);

export async function checkPreimageAuthorization(
  api: any,
  contentHash: Uint8Array
): Promise<Authorization | null> {
  preimageAuthLoadingSubject.next(true);

  try {
    const auth = await api.query.TransactionStorage.Authorizations.getValue(
      Enum("Preimage", Binary.fromBytes(contentHash))
    );

    if (!auth) {
      preimageAuthSubject.next(null);
      return null;
    }

    const authorization: Authorization = {
      transactions: BigInt(auth.extent.transactions),
      bytes: auth.extent.bytes,
      expiresAt: auth.expiration ?? undefined,
    };

    preimageAuthSubject.next(authorization);
    return authorization;
  } catch (err) {
    console.error("Failed to check preimage authorization:", err);
    preimageAuthSubject.next(null);
    return null;
  } finally {
    preimageAuthLoadingSubject.next(false);
  }
}

export function clearPreimageAuth(): void {
  preimageAuthSubject.next(null);
}

// Preimage authorizations (list view for Authorizations page)
const preimageAuthsSubject = new BehaviorSubject<PreimageAuthorization[]>([]);
const preimageAuthsLoadingSubject = new BehaviorSubject<boolean>(false);

export async function fetchPreimageAuthorizations(
  api: any
): Promise<PreimageAuthorization[]> {
  preimageAuthsLoadingSubject.next(true);

  try {
    const entries = await api.query.TransactionStorage.Authorizations.getEntries();

    const preimageAuths: PreimageAuthorization[] = entries
      .filter(({ keyArgs }: any) => keyArgs[0].type === "Preimage")
      .map(({ keyArgs, value }: any) => {
        // Extract content hash from the preimage key
        const preimageValue = keyArgs[0].value;
        let contentHash: Uint8Array;
        if (typeof preimageValue === "object" && preimageValue !== null && "content_hash" in preimageValue) {
          const ch = (preimageValue as { content_hash: { asBytes: () => Uint8Array } }).content_hash;
          contentHash = ch.asBytes();
        } else if (typeof preimageValue === "object" && preimageValue !== null && "asBytes" in preimageValue) {
          contentHash = (preimageValue as { asBytes: () => Uint8Array }).asBytes();
        } else {
          contentHash = new Uint8Array(32);
        }
        return {
          contentHash,
          maxSize: value.extent.bytes,
        };
      });

    preimageAuthsSubject.next(preimageAuths);
    return preimageAuths;
  } catch (err) {
    console.error("Failed to fetch preimage authorizations:", err);
    preimageAuthsSubject.next([]);
    return [];
  } finally {
    preimageAuthsLoadingSubject.next(false);
  }
}

// CID on-chain lookup result
export interface CidOnChainInfo {
  /** Block number where the data was stored */
  blockNumber: number;
  /** Transaction index within the block */
  index: number;
  /** Data size in bytes */
  size: number;
  /** Block number when data expires (blockNumber + retentionPeriod) */
  expiresAtBlock: number;
  /** Current block number at the time of lookup */
  currentBlock: number;
  /** Retention period in blocks */
  retentionPeriod: number;
}

/**
 * Look up a CID on-chain by searching all Transactions entries for a matching content hash.
 * Returns the most recent match (highest block number).
 */
export async function lookupCidOnChain(
  api: any,
  contentHashDigest: Uint8Array,
  currentBlock: number,
): Promise<CidOnChainInfo | null> {
  try {
    const [entries, retentionPeriod] = await Promise.all([
      api.query.TransactionStorage.Transactions.getEntries(),
      api.query.TransactionStorage.RetentionPeriod.getValue(),
    ]);

    const retention = Number(retentionPeriod);
    let bestMatch: CidOnChainInfo | null = null;

    for (const { keyArgs, value } of entries) {
      const blockNum = Number(keyArgs[0]);
      const txInfos: any[] = value;

      for (let idx = 0; idx < txInfos.length; idx++) {
        const info = txInfos[idx];
        const onChainHash: Uint8Array = info.content_hash.asBytes();

        if (onChainHash.length === contentHashDigest.length &&
            onChainHash.every((b: number, i: number) => b === contentHashDigest[i])) {
          const match: CidOnChainInfo = {
            blockNumber: blockNum,
            index: idx,
            size: info.size,
            expiresAtBlock: blockNum + retention,
            currentBlock,
            retentionPeriod: retention,
          };
          // Keep the most recent (highest block number) match — this is the latest
          // upload or renewal.
          if (!bestMatch || blockNum > bestMatch.blockNumber) {
            bestMatch = match;
          }
        }
      }
    }

    return bestMatch;
  } catch (err) {
    console.error("Failed to look up CID on chain:", err);
    return null;
  }
}

// Transaction info by block/index
export async function fetchTransactionInfo(
  api: any,
  blockNumber: number,
  index: number
): Promise<TransactionInfo | null> {
  try {
    const infos = await api.query.TransactionStorage.Transactions.getValue(blockNumber);

    if (!infos || infos.length <= index) {
      return null;
    }

    const info = infos[index];
    if (!info) {
      return null;
    }

    return {
      chunkRoot: info.chunk_root.asBytes(),
      contentHash: info.content_hash.asBytes(),
      size: info.size,
      blockChunks: info.block_chunks,
    };
  } catch (err) {
    console.error("Failed to fetch transaction info:", err);
    return null;
  }
}

// Recent storage events
export interface StorageEvent {
  blockNumber: number;
  blockHash: string;
  index: number;
  who?: string;
  contentHash: Uint8Array;
}

const recentEventsSubject = new BehaviorSubject<StorageEvent[]>([]);

// Auto-refresh authorization when account or API changes
combineLatest([api$, selectedAccount$]).pipe(
  switchMap(([api, account]) => {
    if (!api || !account) {
      authorizationSubject.next(null);
      return of(null);
    }
    return from(fetchAccountAuthorization(api, account.address as SS58String)).pipe(
      catchError(() => of(null))
    );
  })
).subscribe();

// React hooks
export const [useAuthorization] = bind(authorizationSubject, null);
export const [useAuthorizationLoading] = bind(authorizationLoadingSubject, false);
export const [useAuthorizationError] = bind(authorizationErrorSubject, undefined);
export const [usePreimageAuth] = bind(preimageAuthSubject, null);
export const [usePreimageAuthLoading] = bind(preimageAuthLoadingSubject, false);
export const [usePreimageAuthorizations] = bind(preimageAuthsSubject, []);
export const [usePreimageAuthsLoading] = bind(preimageAuthsLoadingSubject, false);
export const [useRecentStorageEvents] = bind(recentEventsSubject, []);

// Direct access
export const authorization$ = authorizationSubject.asObservable();
