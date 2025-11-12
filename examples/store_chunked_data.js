import fs from 'fs'
import { ApiPromise, WsProvider } from '@polkadot/api'
import { Keyring } from '@polkadot/keyring'
import { cryptoWaitReady, blake2AsU8a } from '@polkadot/util-crypto'
import * as multihash from 'multiformats/hashes/digest'
import { CID } from 'multiformats/cid'
import { create } from 'ipfs-http-client'
import * as dagPB from '@ipld/dag-pb'
import * as sha256 from 'multiformats/hashes/sha2';
import { UnixFS } from 'ipfs-unixfs'
import { TextDecoder } from 'util'
import assert from "assert";

// ---- CONFIG ----
const WS_ENDPOINT = 'ws://127.0.0.1:10000' // Bulletin node
const IPFS_API = 'http://127.0.0.1:5001'   // Local IPFS daemon
const HTTP_IPFS_API = 'http://127.0.0.1:8080'   // Local IPFS HTTP gateway
const FILE_PATH = './picture.svg'
const OUT_PATH = './retrieved_picture.bin'
const OUT_PATH2 = './retrieved_picture.bin2'
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

async function fileToDisk(outputPath, fullBuffer) {
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
    const blockSizes = chunks.map(c => BigInt(c.length))
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

async function storeProof(api, sudoPair, pair, rootCID, dagFileBytes, nonceMgr, sudoNonceMgr) {
    console.log(`🧩 Storing proof for rootCID: ${rootCID.toString()} to the Bulletin`);
    // Compute CID manually (same as store() function)
    const proofCid = cidFromBytes(dagFileBytes)

    // Store DAG bytes in Bulletin
    const storeTx = api.tx.transactionStorage.store(to_hex(dagFileBytes));
    const storeResult = await storeTx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement()})
    console.log('📤 DAG proof "bytes" stored in Bulletin:', storeResult.toHuman?.())

    // This can be a serious pallet, this is just a demonstration.
    const proof = `ProofCid: ${proofCid.toString()} -> rootCID: ${rootCID.toString()}`;
    const proofTx = api.tx.system.remark(proof);
    const sudoTx = api.tx.sudo.sudo(proofTx);
    const proofResult = await sudoTx.signAndSend(sudoPair, { nonce: sudoNonceMgr.getAndIncrement()});
    console.log(`📤 DAG proof - "${proof}" - stored in Bulletin:`, proofResult.toHuman?.())
    return { proofCid }
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

async function authorizeStorage(api, sudoPair, pair, nonceMgr) {
    // Ensure enough quota.
    const auth = await api.query.transactionStorage.authorizations({ "Account": pair.address});
    console.log('Authorization info:', auth.toHuman())

    if (!auth.isSome) {
        console.log('ℹ️ No existing authorization found — requesting new one...');
    } else {
        const authValue = auth.unwrap().extent;
        const transactions = authValue.transactions.toNumber();
        const bytes = authValue.bytes.toNumber();

        if (transactions > 10 && bytes > 24 * CHUNK_SIZE) {
            console.log('✅ Account authorization is sufficient.');
            return;
        }
    }

    const transactions = 128;
    const bytes = 64 * 1024 * 1024; // 64 MB
    await authorizeAccount(api, sudoPair, pair.address, transactions, bytes, nonceMgr);
    await waitForNewBlock();
}

async function waitForNewBlock() {
    // TODO: wait for a new block.
    console.log('🛰 Waiting for new block...')
    return new Promise(resolve => setTimeout(resolve, 7000))
}

async function main() {
    await cryptoWaitReady()
    if (fs.existsSync(OUT_PATH)) {
        fs.unlinkSync(OUT_PATH);
        console.log(`File ${OUT_PATH} removed.`);
    }
    if (fs.existsSync(OUT_PATH2)) {
        fs.unlinkSync(OUT_PATH2);
        console.log(`File ${OUT_PATH2} removed.`);
    }

    console.log('🛰 Connecting to Bulletin node...')
    const provider = new WsProvider(WS_ENDPOINT)
    const api = await ApiPromise.create({ provider })
    await api.isReady
    const ipfs = create({ url: IPFS_API });
    console.log('✅ Connected to Bulletin node')

    const keyring = new Keyring({ type: 'sr25519' })
    const pair = keyring.addFromUri('//Alice')
    const sudoPair = keyring.addFromUri('//Alice')
    let { nonce } = await api.query.system.account(pair.address);
    const nonceMgr = new NonceManager(nonce);
    console.log(`💳 Using account: ${pair.address}, nonce: ${nonce}`)

    // Make sure an account can store data.
    await authorizeStorage(api, sudoPair, pair, nonceMgr);

    // Read the file, chunk it, store in Bulletin and return CIDs.
    let { chunks} = await storeChunkedFile(api, pair, FILE_PATH, nonceMgr);
    // Store metadata file with all the CIDs to the Bulletin.
    const { metadataCid} = await storeMetadata(api, pair, chunks, nonceMgr);
    await waitForNewBlock();

    ////////////////////////////////////////////////////////////////////////////////////
    // 1. example manually retrieve the picture (no IPFS DAG feature)
    const metadataJson = await retrieveMetadata(ipfs, metadataCid)
    await retrieveFileForMetadata(ipfs, metadataJson, OUT_PATH);
    filesAreEqual(FILE_PATH, OUT_PATH);

    ////////////////////////////////////////////////////////////////////////////////////
    // 2. example download picture by rootCID with IPFS DAG feature and HTTP gateway.
    // Demonstrates how to download chunked content by one root CID.
    // Basically, just take the `metadataJson` with already stored chunks and convert it to the DAG-PB format.
    const { rootCid, dagBytes } = await buildUnixFSDag(metadataJson)

    // Store DAG proof to the Bulletin.
    let {proofCid} = await storeProof(api, sudoPair, pair, rootCid, Buffer.from(dagBytes), nonceMgr, nonceMgr);
    await waitForNewBlock();
    await reconstructDagFromProof(ipfs, proofCid, rootCid);

    // Store DAG into IPFS.
    // (Alternative: ipfs.dag.put(dagNode, {storeCodec: 'dag-pb', hashAlg: 'sha2-256', pin: true }))
    const dagCid = await ipfs.block.put(dagBytes, {
        format: 'dag-pb',
        mhtype: 'sha2-256'
    })
    assert.strictEqual(
        rootCid.toString(),
        dagCid.toString(),
        '❌ DAG CID does not match expected root CID'
    );
    console.log('🧱 DAG stored on IPFS with CID:', dagCid.toString())
    console.log('\n🌐 Try opening in browser:')
    console.log(`   http://127.0.0.1:8080/ipfs/${rootCid.toString()}`)
    console.log('   (You’ll see binary content since this is an image)')

    // Download the content from IPFS HTTP gateway
    const contentUrl = `${HTTP_IPFS_API}/ipfs/${dagCid.toString()}`;
    console.log('⬇️ Downloading the full content (no chunking) by rootCID from url: ', contentUrl);
    const res = await fetch(contentUrl);
    if (!res.ok) throw new Error(`HTTP error ${res.status}`);
    const fullBuffer = Buffer.from(await res.arrayBuffer());
    console.log(`✅ Reconstructed file size: ${fullBuffer.length} bytes`);
    await fileToDisk(OUT_PATH2, fullBuffer);
    filesAreEqual(FILE_PATH, OUT_PATH2);
    filesAreEqual(OUT_PATH2, OUT_PATH);

    console.log(`\n\n\n✅✅✅ Passed all tests ✅✅✅`);
    await api.disconnect()
}

main().catch(console.error)
