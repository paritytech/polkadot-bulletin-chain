import fs from 'fs'
import { ApiPromise, WsProvider } from '@polkadot/api'
import { Keyring } from '@polkadot/keyring'
import { cryptoWaitReady, blake2AsU8a } from '@polkadot/util-crypto'
import * as multihash from 'multiformats/hashes/digest'
import { CID } from 'multiformats/cid'
import { create } from 'ipfs-http-client'
import * as dagPB from '@ipld/dag-pb'
import { sha256 } from 'multiformats/hashes/sha2'
import { UnixFS } from 'ipfs-unixfs'
import { TextDecoder } from 'util'

// ---- CONFIG ----
const WS_ENDPOINT = 'ws://127.0.0.1:10000' // Bulletin node
const IPFS_API = 'http://127.0.0.1:5001'   // Local IPFS daemon
const FILE_PATH = './picture.svg'
const OUT_PATH = './retrieved_picture.bin'
const CHUNK_SIZE = 4 * 1024 // 4 KB
// -----------------

function to_hex(input) {
    return '0x' + input.toString('hex');
}

async function authorizeAccount(api, pair, who, transactions, bytes, nonceMgr) {
    const tx = api.tx.transactionStorage.authorizeAccount(who, transactions, bytes);
    const sudo_tx = api.tx.sudo.sudo(tx);
    const result = await sudo_tx.signAndSend(pair, {nonce: nonceMgr.getAndIncrement()});
    console.log('Transaction authorizeAccount result:', result.toHuman());
}

/**
 * helper: create CID for raw data
 */
function cidFromBytes(bytes) {
    const hash = blake2AsU8a(bytes)
    const mh = multihash.create(0xb220, hash)
    return CID.createV1(0x55, mh) // 0x55 = raw
}

/**
 * Read the file, chunk it, store in Bulletin and return CIDs.
 * Returns { chunks }
 */
async function storeChunkedFile(api, pair, filePath, nonceMgr) {
    // ---- 1️⃣ Read and split a file ----
    const fileData = fs.readFileSync(filePath)
    console.log(`📁 Read ${filePath}, size ${fileData.length} bytes`)

    const chunks = []
    for (let i = 0; i < fileData.length; i += CHUNK_SIZE) {
        const chunk = fileData.subarray(i, i + CHUNK_SIZE)
        const cid = cidFromBytes(chunk)
        chunks.push({cid, bytes: to_hex(chunk), len: chunk.length})
    }
    console.log(`✂️ Split into ${chunks.length} chunks`)

    // ---- 2️⃣ Store chunks in Bulletin (expecting just one block) ----
    for (let i = 0; i < chunks.length; i++) {
        const {cid, bytes} = chunks[i]
        console.log(`📤 Storing chunk #${i + 1} CID: ${cid}`)
        const tx = api.tx.transactionStorage.store(bytes)
        const result = await tx.signAndSend(pair, {nonce: nonceMgr.getAndIncrement()})
        console.log(`✅ Stored chunk #${i + 1}, result:`, result.toHuman?.())
    }
    return { chunks };
}

/**
 * Reads metadata JSON from IPFS by metadataCid.
 */
async function retrieveMetadata(ipfs, metadataCid) {
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
async function retrieveFileForMetadata(ipfs, metadataJson, outputPath) {
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
        await new Promise((resolve, reject) => {
            const ws = fs.createWriteStream(outputPath);
            ws.write(fullBuffer);
            ws.end();
            ws.on('finish', resolve);
            ws.on('error', reject);
        });
        console.log(`💾 File saved to: ${outputPath}`);
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
            length: c.len
        }))
    };

    const jsonBytes = Buffer.from(new TextEncoder().encode(JSON.stringify(metadata)));
    console.log(`🧾 Metadata size: ${jsonBytes.length} bytes`)

    // 2️⃣ Compute CID manually (same as store() function)
    const metadataCid = cidFromBytes(jsonBytes)
    console.log('🧩 Metadata CID:', metadataCid.toString())

    // 3️⃣ Store JSON bytes in Bulletin
    const tx = api.tx.transactionStorage.store(to_hex(jsonBytes));
    const result = await tx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement()})
    console.log('📤 Metadata stored in Bulletin:', result.toHuman?.())

    return { metadataCid }
}

class NonceManager {
    constructor(initialNonce) {
        this.nonce = initialNonce; // BN instance from api.query.system.account
    }

    getAndIncrement() {
        const current = this.nonce;
        this.nonce = this.nonce.addn(1); // increment BN
        return current;
    }
}

function filesAreEqual(path1, path2) {
    const data1 = fs.readFileSync(path1);
    const data2 = fs.readFileSync(path2);

    if (data1.length !== data2.length) return false;

    for (let i = 0; i < data1.length; i++) {
        if (data1[i] !== data2[i]) return false;
    }
    return true;
}

async function main() {
    await cryptoWaitReady()
    if (fs.existsSync(OUT_PATH)) {
        fs.unlinkSync(OUT_PATH);
        console.log(`File ${OUT_PATH} removed.`);
    }

    console.log('🛰 Connecting to Bulletin node...')
    const provider = new WsProvider(WS_ENDPOINT)
    const api = await ApiPromise.create({ provider })
    await api.isReady
    const ipfs = create({ url: IPFS_API });
    console.log('✅ Connected to Bulletin node')

    const keyring = new Keyring({ type: 'sr25519' })
    const pair = keyring.addFromUri('//Alice')
    const sudo_pair = keyring.addFromUri('//Alice')
    let { nonce } = await api.query.system.account(pair.address);
    const nonceMgr = new NonceManager(nonce);
    console.log(`💳 Using account: ${pair.address}, nonce: ${nonce}`)

    const transactions = 32;
    const bytes = 64 * 1024 * 1024; // 64 MB
    await authorizeAccount(api, sudo_pair, pair.address, transactions, bytes, nonceMgr);

    // Read the file, chunk it, store in Bulletin and return CIDs.
    let { chunks} = await storeChunkedFile(api, pair, FILE_PATH, nonceMgr);
    // Store metadata file with all the CIDs to the Bulletin.
    const { metadataCid} = await storeMetadata(api, pair, chunks, nonceMgr);

    // TODO: wait for a new block.
    await new Promise(resolve => setTimeout(resolve, 7000));

    // 1. example manually retrieve the picture (no IPFS DAG feature)
    const metadataJson = await retrieveMetadata(ipfs, metadataCid)
    await retrieveFileForMetadata(ipfs, metadataJson, OUT_PATH);
    filesAreEqual(FILE_PATH, OUT_PATH);

    // 2. example download picture by rootCID with IPFS DAG feature and HTTP gateway.
    // TODO:

    await api.disconnect()
}

main().catch(console.error)
