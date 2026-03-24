import { createHelia, type Helia } from "helia";
import { unixfs, type UnixFS } from "@helia/unixfs";
import { car } from "@helia/car";
import type { CID } from "multiformats/cid";

export interface CarFileEntry {
  name: string; // Can be a nested path like "css/style.css"
  data: Uint8Array;
}

export interface CarResult {
  carBytes: Uint8Array;
  rootCid: CID;
  files: { name: string; cid: string; size: number }[];
}

// Lazy singleton Helia instance for CAR creation (avoids re-creating libp2p on every call)
let heliaInstance: Helia | null = null;

async function getHelia(): Promise<Helia> {
  if (!heliaInstance) {
    heliaInstance = await createHelia();
  }
  return heliaInstance;
}

// ── Directory tree builder ───────────────────────────────────────────

interface TreeNode {
  files: { name: string; cid: CID }[];
  dirs: Map<string, TreeNode>;
}

/**
 * Recursively build a UnixFS directory from a tree structure.
 * Each node becomes a UnixFS directory containing its files and subdirectories.
 */
async function buildUnixFSTree(fs: UnixFS, node: TreeNode): Promise<CID> {
  let dirCid = await fs.addDirectory();

  for (const file of node.files) {
    dirCid = await fs.cp(file.cid, dirCid, file.name);
  }

  for (const [name, subnode] of node.dirs) {
    const subdirCid = await buildUnixFSTree(fs, subnode);
    dirCid = await fs.cp(subdirCid, dirCid, name);
  }

  return dirCid;
}

// ─────────────────────────────────────────────────────────────────────

/**
 * Create a CAR archive from multiple files.
 * Builds a UnixFS directory tree where each file is addressable by its path,
 * then exports all blocks into a CAR archive.
 *
 * File names can contain "/" to represent nested directory structures
 * (e.g., "css/style.css" creates a "css" subdirectory).
 */
export async function createCarArchive(files: CarFileEntry[]): Promise<CarResult> {
  if (files.length === 0) {
    throw new Error("No files provided");
  }

  const helia = await getHelia();
  const fs = unixfs(helia);
  const heliaCar = car(helia);

  // Add all file bytes and collect CIDs
  const fileCids: { name: string; cid: CID; size: number }[] = [];
  for (const file of files) {
    const fileCid = await fs.addBytes(file.data);
    fileCids.push({ name: file.name, cid: fileCid, size: file.data.length });
  }

  // Build a tree structure from file paths
  const root: TreeNode = { files: [], dirs: new Map() };

  for (const file of fileCids) {
    const parts = file.name.split("/");
    let node = root;

    // Navigate/create intermediate directories
    for (let i = 0; i < parts.length - 1; i++) {
      const dirName = parts[i]!;
      if (!node.dirs.has(dirName)) {
        node.dirs.set(dirName, { files: [], dirs: new Map() });
      }
      node = node.dirs.get(dirName)!;
    }

    // Add file to its parent directory
    node.files.push({ name: parts[parts.length - 1]!, cid: file.cid });
  }

  // Build the UnixFS directory tree
  const rootCid = await buildUnixFSTree(fs, root);

  // Export as CAR - returns AsyncIterable<Uint8Array> with CAR header + blocks
  const parts: Uint8Array[] = [];
  for await (const buf of heliaCar.export(rootCid)) {
    parts.push(buf);
  }

  // Concatenate into single Uint8Array
  const totalLength = parts.reduce((sum, p) => sum + p.length, 0);
  const carBytes = new Uint8Array(totalLength);
  let offset = 0;
  for (const part of parts) {
    carBytes.set(part, offset);
    offset += part.length;
  }

  return {
    carBytes,
    rootCid,
    files: fileCids.map(f => ({ name: f.name, cid: f.cid.toString(), size: f.size })),
  };
}
