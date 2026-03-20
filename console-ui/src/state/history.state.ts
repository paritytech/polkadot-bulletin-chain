import { BehaviorSubject } from "rxjs";
import { bind } from "@react-rxjs/core";

/**
 * Represents a stored data entry that the user has bookmarked
 */
export interface StorageEntry {
  /** Block number where the data was stored */
  blockNumber: number;
  /** Transaction index within the block */
  index: number;
  /** CID string (IPFS-compatible identifier) */
  cid: string;
  /** Content hash (hex string) */
  contentHash: string;
  /** Size in bytes */
  size: number;
  /** Timestamp when the entry was added */
  timestamp: number;
  /** Optional label/description */
  label?: string;
  /** Account that stored this (if known) */
  account?: string;
  /** Network ID where this was stored */
  networkId: string;
}

const STORAGE_KEY = "bulletin-storage-history";

function loadHistory(): StorageEntry[] {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      return JSON.parse(stored);
    }
  } catch (err) {
    console.error("Failed to load storage history:", err);
  }
  return [];
}

function saveHistory(entries: StorageEntry[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(entries));
  } catch (err) {
    console.error("Failed to save storage history:", err);
  }
}

const historySubject = new BehaviorSubject<StorageEntry[]>(loadHistory());

/**
 * Add a new storage entry to history
 */
export function addStorageEntry(entry: Omit<StorageEntry, "timestamp">) {
  const entries = historySubject.getValue();

  // Check if entry already exists (same block + index + network)
  const exists = entries.some(
    (e) => e.blockNumber === entry.blockNumber &&
           e.index === entry.index &&
           e.networkId === entry.networkId
  );

  if (!exists) {
    const newEntry: StorageEntry = {
      ...entry,
      timestamp: Date.now(),
    };
    const updated = [newEntry, ...entries];
    historySubject.next(updated);
    saveHistory(updated);
  }
}

/**
 * Update an existing entry (e.g., add label)
 */
export function updateStorageEntry(
  blockNumber: number,
  index: number,
  networkId: string,
  updates: Partial<Pick<StorageEntry, "label">>
) {
  const entries = historySubject.getValue();
  const updated = entries.map((e) => {
    if (e.blockNumber === blockNumber && e.index === index && e.networkId === networkId) {
      return { ...e, ...updates };
    }
    return e;
  });
  historySubject.next(updated);
  saveHistory(updated);
}

/**
 * Remove an entry from history
 */
export function removeStorageEntry(blockNumber: number, index: number, networkId: string) {
  const entries = historySubject.getValue();
  const updated = entries.filter(
    (e) => !(e.blockNumber === blockNumber && e.index === index && e.networkId === networkId)
  );
  historySubject.next(updated);
  saveHistory(updated);
}

/**
 * Get entries for a specific network
 */
export function getEntriesForNetwork(networkId: string): StorageEntry[] {
  return historySubject.getValue().filter((e) => e.networkId === networkId);
}

/**
 * Get entries for a specific account
 */
export function getEntriesForAccount(account: string): StorageEntry[] {
  return historySubject.getValue().filter((e) => e.account === account);
}

/**
 * Clear all history
 */
export function clearHistory() {
  historySubject.next([]);
  saveHistory([]);
}

// React hooks
export const [useStorageHistory] = bind(historySubject, []);
