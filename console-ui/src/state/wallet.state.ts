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

const STORAGE_KEY_EXTENSION = "bulletin-wallet-extension";
const STORAGE_KEY_ACCOUNT = "bulletin-wallet-account";

export async function connectExtension(extensionName: string): Promise<void> {
  statusSubject.next("connecting");
  errorSubject.next(undefined);

  try {
    const extension = await connectInjectedExtension(extensionName);
    connectedExtensionSubject.next(extension);

    const accounts = extension.getAccounts();
    accountsSubject.next(accounts);

    // Restore previously selected account, or auto-select first
    const savedAddress = localStorage.getItem(STORAGE_KEY_ACCOUNT);
    const savedAccount = savedAddress ? accounts.find(a => a.address === savedAddress) : undefined;
    if (!selectedAccountSubject.getValue()) {
      selectedAccountSubject.next(savedAccount ?? accounts[0]);
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

    localStorage.setItem(STORAGE_KEY_EXTENSION, extensionName);
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
    localStorage.setItem(STORAGE_KEY_ACCOUNT, address);
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
  localStorage.removeItem(STORAGE_KEY_EXTENSION);
  localStorage.removeItem(STORAGE_KEY_ACCOUNT);
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

// Auto-reconnect on page load
export async function restoreWalletConnection(): Promise<void> {
  const savedExtension = localStorage.getItem(STORAGE_KEY_EXTENSION);
  if (!savedExtension) return;

  // Wait briefly for extensions to inject into the page
  await new Promise((resolve) => setTimeout(resolve, 200));

  const available = getInjectedExtensions();
  if (available.includes(savedExtension)) {
    try {
      await connectExtension(savedExtension);
    } catch {
      // Extension no longer available, clear saved state
      localStorage.removeItem(STORAGE_KEY_EXTENSION);
      localStorage.removeItem(STORAGE_KEY_ACCOUNT);
    }
  }
}

// Direct access
export const selectedAccount$ = selectedAccountSubject.asObservable();
export const accounts$ = accountsSubject.asObservable();
