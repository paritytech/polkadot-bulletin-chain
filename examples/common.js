import { CID } from 'multiformats/cid';
import * as dagPB from '@ipld/dag-pb';
import * as sha256 from 'multiformats/hashes/sha2';
import { Keyring } from '@polkadot/keyring';
import { getPolkadotSigner } from '@polkadot-api/signer';
import { createCanvas } from "canvas";
import fs from "fs";
import { TextDecoder } from 'util';
import assert from "assert";
import { cidFromBytes, buildUnixFSDagPB } from "./cid_dag_metadata.js";

// ---- CONFIG ----
export const WS_ENDPOINT = 'ws://127.0.0.1:10000'; // Bulletin node
export const IPFS_API = 'http://127.0.0.1:5001';   // Local IPFS daemon
export const HTTP_IPFS_API = 'http://127.0.0.1:8080';   // Local IPFS HTTP gateway
// -----------------

function to_hex(input) {
  return '0x' + input.toString('hex');
}

async function authorizeAccount(api, pair, who, transactions, bytes, nonceMgr) {
  const tx = api.tx.transactionStorage.authorizeAccount(who, transactions, bytes);
  const sudo_tx = api.tx.sudo.sudo(tx);
  const result = await sudo_tx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement() });
  console.log('Transaction authorizeAccount result:', result.toHuman());
}

/**
 * Read the file, chunk it, store in Bulletin and return CIDs.
 * Returns { chunks }
 */
export async function storeChunkedFile(api, pair, filePath, nonceMgr) {
  // ---- 1️⃣ Read and split a file ----
  const fileData = fs.readFileSync(filePath)
  console.log(`📁 Read ${filePath}, size ${fileData.length} bytes`)

  const chunks = []
  for (let i = 0; i < fileData.length; i += CHUNK_SIZE) {
    const chunk = fileData.subarray(i, i + CHUNK_SIZE)
    const cid = await cidFromBytes(chunk)
    chunks.push({ cid, bytes: to_hex(chunk), len: chunk.length })
  }
  console.log(`✂️ Split into ${chunks.length} chunks`)

  // ---- 2️⃣ Store chunks in Bulletin (expecting just one block) ----
  for (let i = 0; i < chunks.length; i++) {
    const { cid, bytes } = chunks[i]
    console.log(`📤 Storing chunk #${i + 1} CID: ${cid}`)
    try {
      const tx = api.tx.transactionStorage.store(bytes);
      const result = await tx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement() });
      console.log(`✅ Stored chunk #${i + 1}, result:`, result.toHuman?.());
    } catch(err) {
      if (err.stack.includes("Immediately Dropped: The transaction couldn't enter the pool because of the limit")) {
        await waitForNewBlock();
        --i;
        continue;
      }
    }
  }
  return { chunks };
}

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
    console.log(`⬇️  Fetching chunk: ${chunkCid.toString()} (len: ${chunk.len})`);
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
 * Creates and stores metadata describing the file chunks.
 * Returns { metadataCid }
 */
export async function storeMetadata(api, pair, chunks, nonceMgr) {
  // 1️⃣ Prepare JSON metadata (without bytes)
  const metadata = {
    type: 'file',
    version: 1,
    totalChunks: chunks.length,
    totalSize: chunks.reduce((a, c) => a + c.len, 0),
    chunks: chunks.map((c, i) => ({
      index: i,
      cid: c.cid.toString(),
      len: c.len
    }))
  };

  const jsonBytes = Buffer.from(new TextEncoder().encode(JSON.stringify(metadata)));
  console.log(`🧾 Metadata size: ${jsonBytes.length} bytes`)

  // 2️⃣ Compute CID manually (same as store() function)
  const metadataCid = await cidFromBytes(jsonBytes)
  console.log('🧩 Metadata CID:', metadataCid.toString())

  // 3️⃣ Store JSON bytes in Bulletin
  const tx = api.tx.transactionStorage.store(to_hex(jsonBytes));
  const result = await tx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement() })
  console.log('📤 Metadata stored in Bulletin:', result.toHuman?.())

  return { metadataCid }
}

/**
 * Build a UnixFS DAG-PB node for a file composed of chunks.
 * @param {Object} metadataJson - JSON object containing chunks [{ cid, length }]
 * @returns {Promise<{ rootCid: CID, dagBytes: Uint8Array }>}
 */
export async function buildUnixFSDag(metadataJson, mhCode = 0x12) {
    // Extract chunk info
    const chunks = metadataJson.chunks || []
    if (!chunks.length) throw new Error('❌ metadataJson.chunks is empty')

    return await buildUnixFSDagPB(chunks, mhCode);
}

/**
 * Reads a DAG-PB file from IPFS by CID, decodes it, and re-calculates its root CID.
 *
 * @param {object} ipfs - IPFS client (with .block.get)
 * @param {CID|string} proofCid - CID of the stored DAG-PB node
 * @returns {Promise<{ dagNode: any, rootCid: CID }>}
 */
export async function reconstructDagFromProof(ipfs, expectedRootCid, proofCid, mhCode = 0x12) {
  console.log(`📦 Fetching DAG bytes for proof CID: ${proofCid.toString()}`);

  // 1️⃣ Read the raw block bytes from IPFS
  const block = await ipfs.block.get(proofCid);
  const dagBytes = block instanceof Uint8Array ? block : new Uint8Array(block);

  // 2️⃣ Decode the DAG-PB node structure
  const dagNode = dagPB.decode(dagBytes);
  console.log('📄 Decoded DAG node:', dagNode);

  // 3️⃣ Recalculate root CID (same as IPFS does)
  const rootCid = await cidFromBytes(dagBytes, dagPB.code, mhCode);

  assert.strictEqual(
    rootCid.toString(),
    expectedRootCid.toString(),
    '❌ Root DAG CID does not match expected root CID'
  );
  console.log(`✅ Verified reconstructed root CID: ${rootCid.toString()}`);
}

export async function storeProof(api, sudoPair, pair, rootCID, dagFileBytes, nonceMgr, sudoNonceMgr) {
  console.log(`🧩 Storing proof for rootCID: ${rootCID.toString()} to the Bulletin`);
  // Compute CID manually (same as store() function)
  const rawDagCid = await cidFromBytes(dagFileBytes)

  // Store DAG bytes in Bulletin
  const storeTx = api.tx.transactionStorage.store(to_hex(dagFileBytes));
  const storeResult = await storeTx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement() })
  console.log('📤 DAG proof "bytes" stored in Bulletin:', storeResult.toHuman?.())

  // This can be a serious pallet, this is just a demonstration.
  const proof = `ProofCid: ${rawDagCid.toString()} -> rootCID: ${rootCID.toString()}`;
  const proofTx = api.tx.system.remark(proof);
  const sudoTx = api.tx.sudo.sudo(proofTx);
  const proofResult = await sudoTx.signAndSend(sudoPair, { nonce: sudoNonceMgr.getAndIncrement() });
  console.log(`📤 DAG proof - "${proof}" - stored in Bulletin:`, proofResult.toHuman?.())
  return { rawDagCid }
}

export async function waitForNewBlock() {
  // TODO: wait for a new block.
  console.log('🛰 Waiting for new block...')
  return new Promise(resolve => setTimeout(resolve, 7000))
}

/**
 * Creates a PAPI-compatible signer from a Keyring account
 */
export function createSigner(account) {
    return getPolkadotSigner(
        account.publicKey,
        'Sr25519',
        (input) => account.sign(input)
    );
}

export function setupKeyringAndSigners(sudoSeed, accountSeed) {
    const keyring = new Keyring({ type: 'sr25519' });
    const sudoAccount = keyring.addFromUri(sudoSeed);
    const whoAccount = keyring.addFromUri(accountSeed);
    
    const sudoSigner = createSigner(sudoAccount);
    const whoSigner = createSigner(whoAccount);
    
    return {
        sudoSigner,
        whoSigner,
        whoAddress: whoAccount.address
    };
}

/**
 * Generates (dynamic) images based on the input text.
 */
export function generateTextImage(file, text, width = 800, height = 600) {
    const canvas = createCanvas(width, height);
    const ctx = canvas.getContext("2d");

    // 🎨 Background
    ctx.fillStyle = randomColor();
    ctx.fillRect(0, 0, width, height);

    // 🟠 Random shapes
    for (let i = 0; i < 15; i++) {
        ctx.beginPath();
        ctx.fillStyle = randomColor();
        ctx.arc(
            Math.random() * width,
            Math.random() * height,
            Math.random() * 120,
            0,
            Math.PI * 2
        );
        ctx.fill();
    }

    // ✍️ Draw your text
    ctx.font = "bold 40px Sans";
    ctx.fillStyle = "white";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";

    // Add text with shadow for readability
    ctx.shadowColor = "black";
    ctx.shadowBlur = 8;

    ctx.fillText(text, width / 2, height / 2);

    let jpegBytes = canvas.toBuffer("image/jpeg");
    fs.writeFileSync(file, jpegBytes);
    console.log("Saved to file:", file);
}

function randomColor() {
    return `rgb(${rand255()}, ${rand255()}, ${rand255()})`;
}

function rand255() {
    return Math.floor(Math.random() * 256);
}

export function filesAreEqual(path1, path2) {
    const data1 = fs.readFileSync(path1);
    const data2 = fs.readFileSync(path2);
    assert.deepStrictEqual(data1.length, data2.length)

    for (let i = 0; i < data1.length; i++) {
        assert.deepStrictEqual(data1[i], data2[i])
    }
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
