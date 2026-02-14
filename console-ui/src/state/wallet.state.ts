import { BehaviorSubject, combineLatest, map, shareReplay } from "rxjs";
import { bind } from "@react-rxjs/core";
import { connectInjectedExtension, getInjectedExtensions, InjectedExtension, InjectedPolkadotAccount } from "polkadot-api/pjs-signer";

export interface WalletState {
  status: "disconnected" | "connecting" | "connected" | "error";
  error?: string;
  extensions: string[];
  connectedExtension?: InjectedExtension;
  accounts: InjectedPolkadotAccount[];
  selectedAccount?: InjectedPolkadotAccount;
}

const statusSubject = new BehaviorSubject<WalletState["status"]>("disconnected");
const errorSubject = new BehaviorSubject<string | undefined>(undefined);
const extensionsSubject = new BehaviorSubject<string[]>([]);
const connectedExtensionSubject = new BehaviorSubject<InjectedExtension | undefined>(undefined);
const accountsSubject = new BehaviorSubject<InjectedPolkadotAccount[]>([]);
const selectedAccountSubject = new BehaviorSubject<InjectedPolkadotAccount | undefined>(undefined);

export function refreshExtensions(): string[] {
  const extensions = getInjectedExtensions();
  extensionsSubject.next(extensions);
  return extensions;
}

export async function connectExtension(extensionName: string): Promise<void> {
  statusSubject.next("connecting");
  errorSubject.next(undefined);

  try {
    const extension = await connectInjectedExtension(extensionName);
    connectedExtensionSubject.next(extension);

    const accounts = extension.getAccounts();
    accountsSubject.next(accounts);

    // Auto-select first account if available
    if (accounts.length > 0 && !selectedAccountSubject.getValue()) {
      selectedAccountSubject.next(accounts[0]);
    }

    // Subscribe to account changes
    extension.subscribe((newAccounts) => {
      accountsSubject.next(newAccounts);
      // If selected account is no longer available, clear it
      const selected = selectedAccountSubject.getValue();
      if (selected && !newAccounts.find(a => a.address === selected.address)) {
        selectedAccountSubject.next(newAccounts[0]);
      }
    });

    statusSubject.next("connected");
  } catch (err) {
    const message = err instanceof Error ? err.message : "Failed to connect wallet";
    errorSubject.next(message);
    statusSubject.next("error");
    throw err;
  }
}

export function selectAccount(address: string): void {
  const accounts = accountsSubject.getValue();
  const account = accounts.find(a => a.address === address);
  if (account) {
    selectedAccountSubject.next(account);
  }
}

export function disconnectWallet(): void {
  const extension = connectedExtensionSubject.getValue();
  if (extension) {
    extension.disconnect();
  }
  connectedExtensionSubject.next(undefined);
  accountsSubject.next([]);
  selectedAccountSubject.next(undefined);
  statusSubject.next("disconnected");
  errorSubject.next(undefined);
}

// Combined wallet state observable
const walletState$ = combineLatest([
  statusSubject,
  errorSubject,
  extensionsSubject,
  connectedExtensionSubject,
  accountsSubject,
  selectedAccountSubject,
]).pipe(
  map(([status, error, extensions, connectedExtension, accounts, selectedAccount]) => ({
    status,
    error,
    extensions,
    connectedExtension,
    accounts,
    selectedAccount,
  })),
  shareReplay(1)
);

// React hooks
export const [useWalletState] = bind(walletState$, {
  status: "disconnected" as const,
  error: undefined,
  extensions: [],
  connectedExtension: undefined,
  accounts: [],
  selectedAccount: undefined,
});

export const [useWalletStatus] = bind(statusSubject, "disconnected");
export const [useAccounts] = bind(accountsSubject, []);
export const [useSelectedAccount] = bind(selectedAccountSubject, undefined);
export const [useAvailableExtensions] = bind(extensionsSubject, []);

// Direct access
export const selectedAccount$ = selectedAccountSubject.asObservable();
export const accounts$ = accountsSubject.asObservable();
