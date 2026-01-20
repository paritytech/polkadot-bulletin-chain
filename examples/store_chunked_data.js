import fs from 'fs'
import { ApiPromise, WsProvider } from '@polkadot/api'
import { Keyring } from '@polkadot/keyring'
import { cryptoWaitReady } from '@polkadot/util-crypto'
import { CID } from 'multiformats/cid'
import { create } from 'ipfs-http-client'
import * as dagPB from '@ipld/dag-pb'
import { TextDecoder } from 'util'
import assert from "assert";
import { waitForNewBlock, generateTextImage, filesAreEqual, fileToDisk, setupKeyringAndSigners, NonceManager } from './common.js'
import { authorizeAccount, fetchCid, TX_MODE_FINALIZED_BLOCK } from "./api.js";
import { buildUnixFSDagPB, cidFromBytes, convertCid } from "./cid_dag_metadata.js";
import { createClient } from 'polkadot-api';
import {getWsProvider} from "polkadot-api/ws-provider";
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// ---- CONFIG ----
const NODE_WS = 'ws://localhost:10000';
const WS_ENDPOINT = 'ws://127.0.0.1:10000' // Bulletin node
const IPFS_API = 'http://127.0.0.1:5001'   // Local IPFS daemon
const HTTP_IPFS_API = 'http://127.0.0.1:8080'   // Local IPFS HTTP gateway
const FILE_PATH = './random_picture.jpg'
const OUT_1_PATH = './retrieved_random_picture1.jpg'
const OUT_2_PATH = './retrieved_random_picture2.jpg'
const CHUNK_SIZE = 4 * 1024 // 4 KB
// -----------------

function to_hex(input) {
    return '0x' + input.toString('hex');
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
        const cid = await cidFromBytes(chunk)
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
    const result = await tx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement()})
    console.log('📤 Metadata stored in Bulletin:', result.toHuman?.())

    return { metadataCid }
}

/**
 * Build a UnixFS DAG-PB node for a file composed of chunks.
 * @param {Object} metadataJson - JSON object containing chunks [{ cid, length }]
 * @returns {Promise<{ rootCid: CID, dagBytes: Uint8Array }>}
 */
async function buildUnixFSDag(metadataJson, mhCode = 0x12) {
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

async function storeProof(api, sudoPair, pair, rootCID, dagFileBytes, nonceMgr, sudoNonceMgr) {
    console.log(`🧩 Storing proof for rootCID: ${rootCID.toString()} to the Bulletin`);
    // Compute CID manually (same as store() function)
    const rawDagCid = await cidFromBytes(dagFileBytes)

    // Store DAG bytes in Bulletin
    const storeTx = api.tx.transactionStorage.store(to_hex(dagFileBytes));
    const storeResult = await storeTx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement()})
    console.log('📤 DAG proof "bytes" stored in Bulletin:', storeResult.toHuman?.())

    // This can be a serious pallet, this is just a demonstration.
    const proof = `ProofCid: ${rawDagCid.toString()} -> rootCID: ${rootCID.toString()}`;
    const proofTx = api.tx.system.remark(proof);
    const sudoTx = api.tx.sudo.sudo(proofTx);
    const proofResult = await sudoTx.signAndSend(sudoPair, { nonce: sudoNonceMgr.getAndIncrement()});
    console.log(`📤 DAG proof - "${proof}" - stored in Bulletin:`, proofResult.toHuman?.())
    return { rawDagCid }
}

async function main() {
    await cryptoWaitReady()

    let client, api, resultCode;
    try {
        if (fs.existsSync(OUT_1_PATH)) {
            fs.unlinkSync(OUT_1_PATH);
            console.log(`File ${OUT_1_PATH} removed.`);
        }
        if (fs.existsSync(OUT_2_PATH)) {
            fs.unlinkSync(OUT_2_PATH);
            console.log(`File ${OUT_2_PATH} removed.`);
        }
        if (fs.existsSync(FILE_PATH)) {
            fs.unlinkSync(FILE_PATH);
            console.log(`File ${FILE_PATH} removed.`);
        }
        generateTextImage(FILE_PATH, "Hello, Bulletin with PAPI chunked - " + new Date().toString(), "small");

        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);
        const { sudoSigner, whoSigner, whoAddress } = setupKeyringAndSigners('//Alice', '//Chunkedsigner');

        // Authorize an account.
        await authorizeAccount(
            bulletinAPI,
            sudoSigner,
            whoAddress,
            100,
            BigInt(100 * 1024 * 1024), // 100 MiB
            TX_MODE_FINALIZED_BLOCK
        );

        console.log('🛰 Connecting to Bulletin node...')
        const provider = new WsProvider(WS_ENDPOINT)
        api = await ApiPromise.create({ provider })
        await api.isReady
        const ipfs = create({ url: IPFS_API });
        console.log('✅ Connected to Bulletin node')

        const keyring = new Keyring({ type: 'sr25519' })
        const pair = keyring.addFromUri('//Alice')
        const sudoPair = keyring.addFromUri('//Alice')
        let { nonce } = await api.query.system.account(pair.address);
        const nonceMgr = new NonceManager(nonce);
        console.log(`💳 Using account: ${pair.address}, nonce: ${nonce}`)

        // Read the file, chunk it, store in Bulletin and return CIDs.
        let { chunks} = await storeChunkedFile(api, pair, FILE_PATH, nonceMgr);
        // Store metadata file with all the CIDs to the Bulletin.
        const { metadataCid} = await storeMetadata(api, pair, chunks, nonceMgr);
        await waitForNewBlock();

        ////////////////////////////////////////////////////////////////////////////////////
        // 1. example manually retrieve the picture (no IPFS DAG feature)
        const metadataJson = await retrieveMetadata(ipfs, metadataCid)
        await retrieveFileForMetadata(ipfs, metadataJson, OUT_1_PATH);
        filesAreEqual(FILE_PATH, OUT_1_PATH);

        ////////////////////////////////////////////////////////////////////////////////////
        // 2. example download picture by rootCID with IPFS DAG feature and HTTP gateway.
        // Demonstrates how to download chunked content by one root CID.
        // Basically, just take the `metadataJson` with already stored chunks and convert it to the DAG-PB format.
        const { rootCid, dagBytes } = await buildUnixFSDag(metadataJson, 0xb220)

        // Store DAG and proof to the Bulletin.
        let { rawDagCid } = await storeProof(api, sudoPair, pair, rootCid, Buffer.from(dagBytes), nonceMgr, nonceMgr);
        await waitForNewBlock();
        await reconstructDagFromProof(ipfs, rootCid, rawDagCid, 0xb220);

        // Store DAG into IPFS.
        assert.strictEqual(
            rootCid.toString(),
            convertCid(rawDagCid, dagPB.code).toString(),
            '❌ DAG CID does not match expected root CID'
        );
        console.log('🧱 DAG stored on IPFS with CID:', rawDagCid.toString())
        console.log('\n🌐 Try opening in browser:')
        console.log(`   http://127.0.0.1:8080/ipfs/${rootCid.toString()}`)
        console.log('   (You’ll see binary content since this is an image)')
        console.log(`   http://127.0.0.1:8080/ipfs/${rawDagCid.toString()}`)
        console.log('   (You’ll see the encoded DAG descriptor content)')

        // Download the content from IPFS HTTP gateway
        const fullBuffer = await fetchCid(HTTP_IPFS_API, rootCid);
        console.log(`✅ Reconstructed file size: ${fullBuffer.length} bytes`);
        await fileToDisk(OUT_2_PATH, fullBuffer);
        filesAreEqual(FILE_PATH, OUT_1_PATH);
        filesAreEqual(OUT_1_PATH, OUT_2_PATH);

        // Download the DAG descriptor raw file itself.
        const downloadedDagBytes = await fetchCid(HTTP_IPFS_API, rawDagCid);
        console.log(`✅ Downloaded DAG raw descriptor file size: ${downloadedDagBytes.length} bytes`);
        assert.deepStrictEqual(downloadedDagBytes, Buffer.from(dagBytes));
        const dagNode = dagPB.decode(downloadedDagBytes);
        console.log('📄 Decoded DAG node:', dagNode);

        console.log(`\n\n\n✅✅✅ Test passed! ✅✅✅`);
        resultCode = 0;
    } catch (error) {
        console.error("❌ Error:", error);
        resultCode = 1;
    } finally {
        if (api) api.disconnect();
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
