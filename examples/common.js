import fs from 'fs';
import { CID } from 'multiformats/cid';
import * as dagPB from '@ipld/dag-pb';
import * as sha256 from 'multiformats/hashes/sha2';
import { UnixFS } from 'ipfs-unixfs';
import { TextDecoder } from 'util';
import assert from "assert";

// ---- CONFIG ----
export const WS_ENDPOINT = 'ws://127.0.0.1:10000'; // Bulletin node
export const IPFS_API = 'http://127.0.0.1:5001';   // Local IPFS daemon
export const HTTP_IPFS_API = 'http://127.0.0.1:8080';   // Local IPFS HTTP gateway
// -----------------

/**
 * Reads metadata JSON from IPFS by metadataCid.
 */
export async function retrieveMetadata(ipfs, metadataCid) {
  console.log(`🧩 Retrieving file from metadataCid: ${metadataCid.toString()}`);

  // 1️⃣ Fetch metadata block
  const metadataBlock = await ipfs.block.get(metadataCid);
  const metadataJson = JSON.parse(new TextDecoder().decode(metadataBlock));
  console.log(`📜 Loaded metadata:`, metadataJson);
  return metadataJson;
}

export async function fileToDisk(outputPath, fullBuffer) {
  await new Promise((resolve, reject) => {
    const ws = fs.createWriteStream(outputPath);
    ws.write(fullBuffer);
    ws.end();
    ws.on('finish', resolve);
    ws.on('error', reject);
  });
  console.log(`💾 File saved to: ${outputPath}`);
}

/**
 * Fetches all chunks listed in metdataJson, concatenates into a single file,
 * and saves to disk (or returns as Buffer).
 */
export async function retrieveFileForMetadata(ipfs, metadataJson, outputPath) {
  console.log(`🧩 Retrieving file for metadataJson`);

  // Basic sanity check
  if (!metadataJson.chunks || !Array.isArray(metadataJson.chunks)) {
    throw new Error('Invalid metadata: no "chunks" array found');
  }

  // 2️⃣ Fetch each chunk by CID
  const buffers = [];
  for (const chunk of metadataJson.chunks) {
    const chunkCid = CID.parse(chunk.cid);
    console.log(`⬇️  Fetching chunk: ${chunkCid.toString()} (len: ${chunk.length})`);
    const block = await ipfs.block.get(chunkCid);
    buffers.push(block);
  }

  // 3️⃣ Concatenate into a single buffer
  const fullBuffer = Buffer.concat(buffers);
  console.log(`✅ Reconstructed file size: ${fullBuffer.length} bytes`);

  // 4️⃣ Optionally save to disk
  if (outputPath) {
    await fileToDisk(outputPath, fullBuffer);
  }

  return fullBuffer;
}

/**
 * Build a UnixFS DAG-PB node for a file composed of chunks.
 * @param {Object} metadataJson - JSON object containing chunks [{ cid, length }]
 * @returns {Promise<{ rootCid: CID, dagBytes: Uint8Array }>}
 */
export async function buildUnixFSDag(metadataJson) {
  // Extract chunk info
  const chunks = metadataJson.chunks || []
  if (!chunks.length) throw new Error('❌ metadataJson.chunks is empty')

  // Prepare UnixFS file metadata
  const blockSizes = chunks.map(c => Number(c.length))
  const fileData = new UnixFS({ type: 'file', blockSizes })

  console.log(`\n🧩 Building UnixFS DAG:
  • totalChunks: ${chunks.length}
  • blockSizes: ${blockSizes.join(', ')}`)

  // Prepare DAG-PB node
  const dagNode = dagPB.prepare({
    Data: fileData.marshal(),
    Links: chunks.map(c => ({
      Name: '',
      Tsize: c.length,
      Hash: c.cid
    }))
  })

  // Encode and hash to create dag root CID.
  const dagBytes = dagPB.encode(dagNode)
  const dagHash = await sha256.sha256.digest(dagBytes)
  const rootCid = CID.createV1(dagPB.code, dagHash)

  console.log(`✅ Built DAG root CID: ${rootCid.toString()}`)
  return { rootCid, dagBytes }
}

/**
 * Reads a DAG-PB file from IPFS by CID, decodes it, and re-calculates its root CID.
 *
 * @param {object} ipfs - IPFS client (with .block.get)
 * @param {CID|string} proofCid - CID of the stored DAG-PB node
 * @returns {Promise<{ dagNode: any, rootCid: CID }>}
 */
export async function reconstructDagFromProof(ipfs, proofCid, expectedRootCid) {
  console.log(`📦 Fetching DAG bytes for proof CID: ${proofCid.toString()}`);

  // 1️⃣ Read the raw block bytes from IPFS
  const block = await ipfs.block.get(proofCid);
  const dagBytes = block instanceof Uint8Array ? block : new Uint8Array(block);

  // 2️⃣ Decode the DAG-PB node structure
  const dagNode = dagPB.decode(dagBytes);
  console.log('📄 Decoded DAG node:', dagNode);

  // 3️⃣ Recalculate root CID (same as IPFS does)
  const hash = await sha256.sha256.digest(dagBytes);
  const rootCid = CID.createV1(dagPB.code, hash);

  assert.strictEqual(
    rootCid.toString(),
    expectedRootCid.toString(),
    '❌ Root DAG CID does not match expected root CID'
  );
  console.log(`✅ Verified reconstructed root CID: ${rootCid.toString()}`);
}

export class NonceManager {
  constructor(initialNonce) {
    this.nonce = initialNonce; // BN instance from api.query.system.account
  }

  getAndIncrement() {
    const current = this.nonce;
    this.nonce = this.nonce.addn(1); // increment BN
    return current;
  }
}

export function filesAreEqual(path1, path2) {
  const data1 = fs.readFileSync(path1);
  const data2 = fs.readFileSync(path2);

  if (data1.length !== data2.length) return false;

  for (let i = 0; i < data1.length; i++) {
    if (data1[i] !== data2[i]) return false;
  }
  return true;
}

export async function waitForNewBlock() {
  // TODO: wait for a new block.
  console.log('🛰 Waiting for new block...')
  return new Promise(resolve => setTimeout(resolve, 7000))
}
