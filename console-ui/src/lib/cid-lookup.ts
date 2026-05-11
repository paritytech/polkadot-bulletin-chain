import type { CID } from "@parity/bulletin-sdk";
import { CidCodec, UnixFsDagBuilder } from "@parity/bulletin-sdk";
import { Binary, type HexString } from "polkadot-api";

/** Chain truth for a single stored transaction. */
export interface OnChainTransaction {
  blockNumber: number;
  index: number;
  size: number;
}

/** A CID resolved (or attempted) against the chain. */
export interface CidResolution {
  cid: CID;
  cidString: string;
  /** Multihash digest as 0x-prefixed hex; matches on-chain `content_hash`. */
  contentHashHex: string;
  /** True if this CID is the DAG-PB root manifest, false for chunks or raw CIDs. */
  isManifest: boolean;
  /** null if no on-chain transaction matches this CID's content hash. */
  location: OnChainTransaction | null;
}

export function isDagPb(cid: CID): boolean {
  return cid.code === CidCodec.DagPb;
}

/** 32-byte multihash digest as 0x-hex — the on-chain `content_hash` shape. */
export function contentHashHex(cid: CID): string {
  return Binary.toHex(cid.multihash.digest);
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Api = any;

interface TxInfo {
  content_hash: HexString;
  size: number;
}

async function locateContentHashes(
  api: Api,
  hashes: HexString[],
): Promise<Map<HexString, OnChainTransaction | null>> {
  const out = new Map<HexString, OnChainTransaction | null>();
  if (hashes.length === 0) return out;
  for (const h of hashes) out.set(h, null);

  const indexHits = (await Promise.all(
    hashes.map((h) =>
      api.query.TransactionStorage.TransactionByContentHash.getValue(h),
    ),
  )) as Array<[number | bigint, number] | undefined>;

  const uniqueBlocks = new Set<number>();
  const found: Array<{ hash: HexString; block: number; index: number }> = [];
  for (let i = 0; i < hashes.length; i++) {
    const hit = indexHits[i];
    if (!hit) continue;
    const block = Number(hit[0]);
    const index = hit[1];
    found.push({ hash: hashes[i]!, block, index });
    uniqueBlocks.add(block);
  }
  if (found.length === 0) return out;

  const blocks = Array.from(uniqueBlocks);
  const blockInfos = (await Promise.all(
    blocks.map((b) => api.query.TransactionStorage.Transactions.getValue(b)),
  )) as Array<TxInfo[] | undefined>;
  const blockMap = new Map<number, TxInfo[]>();
  blocks.forEach((b, i) => blockMap.set(b, blockInfos[i] ?? []));

  for (const f of found) {
    const info = blockMap.get(f.block)?.[f.index];
    if (!info) continue;
    out.set(f.hash, { blockNumber: f.block, index: f.index, size: info.size });
  }
  return out;
}

/**
 * Look up a single CID on chain. Returns null if not found.
 */
export async function lookupCidOnChain(
  api: Api,
  cid: CID,
): Promise<OnChainTransaction | null> {
  const hash = contentHashHex(cid) as HexString;
  const map = await locateContentHashes(api, [hash]);
  return map.get(hash) ?? null;
}

export interface ResolveCidOptions {
  /**
   * Pre-known `content_hash` → location pairs (e.g. browser upload history).
   * CIDs covered by hints skip the on-chain lookup entirely; if hints cover
   * all CIDs we never call the chain.
   */
  localHints?: Map<string, OnChainTransaction>;
  onProgress?: (phase: "fetch-manifest" | "lookup-chain") => void;
}

export interface ResolveCidResult {
  /** Manifest first (if DAG-PB), then chunks in declared order. */
  resolutions: CidResolution[];
  /** UnixFs total file size for DAG-PB; otherwise the single resolution's size (or 0). */
  totalSize: number;
}

/**
 * Resolve a root CID to all on-chain transactions backing it.
 *
 * - Raw CID: returns one resolution.
 * - DAG-PB CID: fetches the manifest via `fetchRawBlock`, parses it with
 *   `UnixFsDagBuilder`, returns `[manifest, ...chunks]`.
 *
 * `fetchRawBlock` is injected so this module stays transport-agnostic
 * (browser gateway today, bitswap or test mock later).
 */
export async function resolveCid(
  api: Api,
  rootCid: CID,
  fetchRawBlock: (cidStr: string) => Promise<Uint8Array>,
  opts: ResolveCidOptions = {},
): Promise<ResolveCidResult> {
  const { localHints, onProgress } = opts;

  let targets: { cid: CID; isManifest: boolean }[];
  let manifestTotalSize: number | null = null;

  if (isDagPb(rootCid)) {
    onProgress?.("fetch-manifest");
    const manifestBytes = await fetchRawBlock(rootCid.toString());
    const { chunkCids, totalSize } = await new UnixFsDagBuilder().parse(manifestBytes);
    manifestTotalSize = totalSize;
    targets = [
      { cid: rootCid, isManifest: true },
      ...chunkCids.map((c: CID) => ({ cid: c, isManifest: false })),
    ];
  } else {
    targets = [{ cid: rootCid, isManifest: false }];
  }

  const enriched = targets.map(({ cid, isManifest }) => ({
    cid,
    cidString: cid.toString(),
    contentHashHex: contentHashHex(cid),
    isManifest,
  }));

  const locations = new Map<string, OnChainTransaction | null>();
  const misses: HexString[] = [];
  for (const e of enriched) {
    const hint = localHints?.get(e.contentHashHex);
    if (hint) {
      locations.set(e.contentHashHex, hint);
    } else {
      misses.push(e.contentHashHex as HexString);
    }
  }

  if (misses.length > 0) {
    onProgress?.("lookup-chain");
    const located = await locateContentHashes(api, misses);
    for (const hash of misses) {
      locations.set(hash, located.get(hash) ?? null);
    }
  }

  const resolutions: CidResolution[] = enriched.map((e) => ({
    cid: e.cid,
    cidString: e.cidString,
    contentHashHex: e.contentHashHex,
    isManifest: e.isManifest,
    location: locations.get(e.contentHashHex) ?? null,
  }));

  const totalSize = manifestTotalSize ?? resolutions[0]?.location?.size ?? 0;

  return { resolutions, totalSize };
}

