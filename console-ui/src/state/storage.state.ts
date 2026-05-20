import { BehaviorSubject, combineLatest, switchMap, of, from, catchError } from "rxjs";
import { bind } from "@react-rxjs/core";
import { api$ } from "./chain.state";
import { selectedAccount$ } from "./wallet.state";
import { SS58String, Enum, Binary, type HexString } from "polkadot-api";

export interface Authorization {
  // Remaining quota; used by Upload precheck and similar flows.
  transactions: bigint;
  bytes: bigint;
  expiresAt?: number;
  // Raw consumed values straight from the on-chain extent, for display.
  used: {
    transactions: bigint;
    bytesEphemeral: bigint;
    bytesPermanent: bigint;
  };
  // Raw caps from the extent, for display.
  allowance: {
    transactions: bigint;
    bytes: bigint;
  };
}

export interface PreimageAuthorization {
  contentHash: Uint8Array;
  maxSize: bigint;
}

/**
 * Backwards-compatible read of an `AuthorizationExtent`: newer chains track
 * consumption separately (`*_allowance` for the cap, `transactions`/`bytes` for
 * usage); older chains expose only the cap. The UI surfaces "remaining"
 * everywhere, so we compute `allowance - consumed` when both are present and
 * fall back to the raw value otherwise.
 */
export function extentRemainingTransactions(extent: any): bigint {
  const allowance = extent?.transactions_allowance;
  if (allowance != null) {
    const used = BigInt(extent.transactions ?? 0);
    const cap = BigInt(allowance);
    return cap > used ? cap - used : 0n;
  }
  return BigInt(extent?.transactions ?? 0);
}

export function extentRemainingBytes(extent: any): bigint {
  const allowance = extent?.bytes_allowance;
  if (allowance != null) {
    const used = BigInt(extent.bytes ?? 0n);
    const cap = BigInt(allowance);
    return cap > used ? cap - used : 0n;
  }
  return BigInt(extent?.bytes ?? 0n);
}

export function extentAllowanceBytes(extent: any): bigint {
  const allowance = extent?.bytes_allowance;
  return BigInt(allowance ?? extent?.bytes ?? 0n);
}

export function extentAllowanceTransactions(extent: any): bigint {
  const allowance = extent?.transactions_allowance;
  return BigInt(allowance ?? extent?.transactions ?? 0);
}

function buildAuthorization(extent: any, expiration: number | null | undefined): Authorization {
  return {
    transactions: extentRemainingTransactions(extent),
    bytes: extentRemainingBytes(extent),
    expiresAt: expiration ?? undefined,
    used: {
      transactions: BigInt(extent?.transactions ?? 0),
      bytesEphemeral: BigInt(extent?.bytes ?? 0n),
      bytesPermanent: BigInt(extent?.bytes_permanent ?? 0n),
    },
    allowance: {
      transactions: extentAllowanceTransactions(extent),
      bytes: extentAllowanceBytes(extent),
    },
  };
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

    const authorization = buildAuthorization(auth.extent, auth.expiration);

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
      Enum("Preimage", Binary.toHex(contentHash))
    );

    if (!auth) {
      preimageAuthSubject.next(null);
      return null;
    }

    const authorization = buildAuthorization(auth.extent, auth.expiration);

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
        const preimageValue = keyArgs[0].value;
        const contentHash =
          typeof preimageValue === "string"
            ? Binary.fromHex(preimageValue as HexString)
            : new Uint8Array(32);
        return {
          contentHash,
          maxSize: extentAllowanceBytes(value.extent),
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
      chunkRoot: Binary.fromHex(info.chunk_root as HexString),
      contentHash: Binary.fromHex(info.content_hash as HexString),
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
