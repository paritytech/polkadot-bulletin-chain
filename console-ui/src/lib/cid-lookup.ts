import type { CID } from "@parity/bulletin-sdk";
import { CidCodec, UnixFsDagBuilder } from "@parity/bulletin-sdk";
import { bulletin_westend } from "@polkadot-api/descriptors";
import { Binary, type HexString, type SizedHex, type TypedApi } from "polkadot-api";

/** Chain truth for a single stored transaction. */
export interface OnChainTransaction {
  blockNumber: number;
  index: number;
  size?: number;
  kind?: "Store" | "Renew";
  cidCodec?: number;
  hashing?: string;
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

type Api = TypedApi<typeof bulletin_westend>;

async function locateContentHashes(
  api: Api,
  hashes: HexString[],
): Promise<Map<HexString, OnChainTransaction | null>> {
  const out = new Map<HexString, OnChainTransaction | null>();
  if (hashes.length === 0) return out;
  for (const h of hashes) out.set(h, null);

  const indexHits = await Promise.all(
    hashes.map((h) =>
      api.query.TransactionStorage.TransactionByContentHash.getValue(
        h as SizedHex<32>,
      ),
    ),
  );

  const infoHits = await Promise.all(
    indexHits.map((hit) =>
      hit
        ? api.query.TransactionStorage.Transactions.getValue(hit[0]!).catch(() => null)
        : Promise.resolve(null),
    ),
  );

  for (let i = 0; i < hashes.length; i++) {
    const hit = indexHits[i];
    if (!hit) continue;
    const blockNumber = hit[0]!;
    const txIndex = hit[1]!;
    const infos = infoHits[i];
    const info = Array.isArray(infos) ? infos[txIndex] : undefined;
    out.set(hashes[i]!, {
      blockNumber,
      index: txIndex,
      size: info?.size != null ? Number(info.size) : undefined,
      kind: info?.kind?.type,
      cidCodec: info?.cid_codec != null ? Number(info.cid_codec) : undefined,
      hashing: info?.hashing?.type,
    });
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
  /** UnixFs total file size from the DAG-PB manifest; 0 for raw CIDs (no manifest to aggregate from). */
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

  const totalSize = manifestTotalSize ?? 0;

  return { resolutions, totalSize };
}

