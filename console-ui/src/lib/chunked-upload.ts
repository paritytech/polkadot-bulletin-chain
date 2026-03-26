import { cidFromBytes } from "@/lib/cid";
import { CID } from "multiformats/cid";
import * as dagPB from "@ipld/dag-pb";
import { UnixFS } from "ipfs-unixfs";

export const CHUNK_SIZE = 1 * 1024 * 1024; // 1 MiB
export const CHUNKED_THRESHOLD = 2 * 1024 * 1024; // 2 MB

export interface ChunkInfo {
  cid: CID;
  data: Uint8Array;
  size: number;
}

export interface DagResult {
  rootCid: CID;
  dagBytes: Uint8Array;
}

/**
 * Split data into fixed-size chunks and compute CID for each.
 */
export async function chunkData(
  data: Uint8Array,
  codec: number,
  hashCode: number,
  chunkSize: number = CHUNK_SIZE,
): Promise<ChunkInfo[]> {
  const chunks: ChunkInfo[] = [];
  for (let i = 0; i < data.length; i += chunkSize) {
    const chunk = data.slice(i, i + chunkSize);
    const cid = await cidFromBytes(chunk, codec, hashCode);
    chunks.push({ cid, data: chunk, size: chunk.length });
  }
  return chunks;
}

/**
 * Build metadata JSON describing the file chunks.
 * Follows the same format as store_chunked_data.js for compatibility.
 */
export function buildChunkMetadata(
  chunks: { cid: string; size: number }[],
  totalSize: number,
): Uint8Array {
  const metadata = {
    type: "file",
    version: 1,
    totalChunks: chunks.length,
    totalSize,
    chunks: chunks.map((c, i) => ({
      index: i,
      cid: c.cid,
      len: c.size,
    })),
  };
  return new TextEncoder().encode(JSON.stringify(metadata));
}

/**
 * Build a UnixFS DAG-PB file node that links all chunks together.
 * When stored on-chain with dag-pb codec, the root CID becomes
 * directly accessible on IPFS — it can traverse the links to fetch
 * and reassemble all chunks into the original file.
 *
 * Ported from examples/cid_dag_metadata.js:buildUnixFSDagPB()
 */
export async function buildUnixFSDag(
  chunks: { cid: string; size: number }[],
  hashCode: number,
): Promise<DagResult> {
  if (!chunks.length) {
    throw new Error("buildUnixFSDag: chunks[] is empty");
  }

  // UnixFS blockSizes = sizes of child blocks
  const blockSizes = chunks.map(c => BigInt(c.size));

  // Build UnixFS file metadata (no inline data — data lives in linked chunks)
  const fileData = new UnixFS({
    type: "file",
    blockSizes,
  });

  // DAG-PB node: file descriptor with chunk links
  const dagNode = dagPB.prepare({
    Data: fileData.marshal(),
    Links: chunks.map(c => ({
      Name: "",
      Tsize: c.size,
      Hash: CID.parse(c.cid),
    })),
  });

  // Encode DAG-PB and compute CID with dag-pb codec (0x70)
  const dagBytes = dagPB.encode(dagNode);
  const rootCid = await cidFromBytes(dagBytes, dagPB.code, hashCode);

  return { rootCid, dagBytes };
}
