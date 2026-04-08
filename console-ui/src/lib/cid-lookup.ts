import type { CID } from "@parity/bulletin-sdk";
import { CidCodec } from "@parity/bulletin-sdk";
import type { StorageEntry } from "@/state/history.state";
import { bytesToHex } from "@/utils/format";

/**
 * Result of resolving a CID to its on-chain location.
 */
export interface CidResolution {
  cid: CID;
  cidString: string;
  contentHash: Uint8Array;
  blockNumber: number | null;
  index: number | null;
  found: boolean;
  isManifest: boolean;
}

/**
 * Extract the 32-byte content hash (multihash digest) from a CID.
 * This matches the on-chain ContentHash = [u8; 32].
 */
export function contentHashFromCid(cid: CID): Uint8Array {
  return cid.multihash.digest;
}

/**
 * Check if a CID uses DAG-PB codec (0x70).
 */
export function isDagPb(cid: CID): boolean {
  return cid.code === CidCodec.DagPb;
}

/**
 * Look up a content hash in local browser history.
 */
function lookupInHistory(
  contentHashHex: string,
  history: StorageEntry[],
): { blockNumber: number; index: number } | null {
  const entry = history.find((e) => e.contentHash === contentHashHex);
  if (entry) {
    return { blockNumber: entry.blockNumber, index: entry.index };
  }
  return null;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
interface TransactionEntry {
  keyArgs: [number];
  value: Array<{
    content_hash: { asBytes(): Uint8Array };
    size: number;
  }>;
}

/**
 * Resolve multiple CIDs to their on-chain block/index locations.
 *
 * Optimized to call Transactions.getEntries() only once and scan the result
 * for all CIDs. Checks local history first for each CID before falling back
 * to the chain scan.
 */
export async function resolveAllCids(
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  api: any,
  cids: { cid: CID; isManifest: boolean }[],
  networkHistory: StorageEntry[],
  onProgress?: (resolved: number, total: number) => void,
): Promise<CidResolution[]> {
  const total = cids.length;
  const results: CidResolution[] = [];

  // Build content hash map for all CIDs
  const cidInfos = cids.map(({ cid, isManifest }) => {
    const contentHash = contentHashFromCid(cid);
    const contentHashHex = bytesToHex(contentHash);
    return { cid, isManifest, contentHash, contentHashHex, cidString: cid.toString() };
  });

  // Try history first for each CID
  const needsChainLookup: typeof cidInfos = [];
  for (const info of cidInfos) {
    const historyResult = lookupInHistory(info.contentHashHex, networkHistory);
    if (historyResult) {
      results.push({
        cid: info.cid,
        cidString: info.cidString,
        contentHash: info.contentHash,
        blockNumber: historyResult.blockNumber,
        index: historyResult.index,
        found: true,
        isManifest: info.isManifest,
      });
    } else {
      needsChainLookup.push(info);
    }
  }

  onProgress?.(results.length, total);

  // If all resolved from history, return early
  if (needsChainLookup.length === 0) {
    return results;
  }

  // Fetch all transaction entries from chain (single RPC call)
  const entries: TransactionEntry[] =
    await api.query.TransactionStorage.Transactions.getEntries();

  // Build a lookup map: contentHashHex -> { blockNumber, index }
  const chainMap = new Map<string, { blockNumber: number; index: number }>();
  for (const { keyArgs, value } of entries) {
    const blockNumber = Number(keyArgs[0]);
    if (!Array.isArray(value)) continue;
    for (let idx = 0; idx < value.length; idx++) {
      const info = value[idx];
      if (!info) continue;
      const hash = bytesToHex(info.content_hash.asBytes());
      // Only store first occurrence (earliest block)
      if (!chainMap.has(hash)) {
        chainMap.set(hash, { blockNumber, index: idx });
      }
    }
  }

  // Resolve remaining CIDs from chain data
  for (const info of needsChainLookup) {
    const chainResult = chainMap.get(info.contentHashHex);
    results.push({
      cid: info.cid,
      cidString: info.cidString,
      contentHash: info.contentHash,
      blockNumber: chainResult?.blockNumber ?? null,
      index: chainResult?.index ?? null,
      found: !!chainResult,
      isManifest: info.isManifest,
    });
    onProgress?.(results.length, total);
  }

  // Sort results: manifest first, then chunks in order
  results.sort((a, b) => {
    if (a.isManifest && !b.isManifest) return 0;
    if (!a.isManifest && b.isManifest) return 0;
    return 0;
  });

  // Preserve original order (manifest first, then chunks)
  const ordered: CidResolution[] = [];
  for (const { cid } of cids) {
    const match = results.find((r) => r.cidString === cid.toString());
    if (match) ordered.push(match);
  }

  return ordered;
}
