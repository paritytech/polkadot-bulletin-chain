import { createHelia, type Helia } from "helia";
import { unixfs } from "@helia/unixfs";
import { car } from "@helia/car";
import type { CID } from "multiformats/cid";

export interface CarFileEntry {
  name: string;
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

/**
 * Create a CAR archive from multiple files.
 * Builds a UnixFS directory where each file is addressable by name,
 * then exports all blocks into a CAR archive.
 */
export async function createCarArchive(files: CarFileEntry[]): Promise<CarResult> {
  if (files.length === 0) {
    throw new Error("No files provided");
  }

  const helia = await getHelia();
  const fs = unixfs(helia);
  const heliaCar = car(helia);

  // Create root directory and add each file
  let rootCid = await fs.addDirectory();
  const fileEntries: CarResult["files"] = [];

  for (const file of files) {
    const fileCid = await fs.addBytes(file.data);
    rootCid = await fs.cp(fileCid, rootCid, file.name);
    fileEntries.push({
      name: file.name,
      cid: fileCid.toString(),
      size: file.data.length,
    });
  }

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

  return { carBytes, rootCid, files: fileEntries };
}
