import fs from 'fs'
import os from 'os'
import path from 'path'
import { cryptoWaitReady } from '@polkadot/util-crypto'
import { CID } from 'multiformats/cid'
import * as dagPB from '@ipld/dag-pb'
import { TextDecoder } from 'util'
import assert from "assert";
import { generateTextImage, filesAreEqual, fileToDisk, setupKeyringAndSigners, DEFAULT_IPFS_GATEWAY_URL } from './common.js'
import { logHeader, logConnection, logSuccess, logError, logTestResult } from './logger.js'
import { authorizeAccount, fetchCid, store, storeChunkedFile, waitForTransaction, TX_MODE_FINALIZED_BLOCK } from "./api.js";
import { buildUnixFSDagPB, cidFromBytes, convertCid } from "./cid_dag_metadata.js";
import { createClient } from 'polkadot-api';
import { getWsProvider } from "polkadot-api/ws-provider";
import { Binary } from '@polkadot-api/substrate-bindings';
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Command line arguments: [ws_url] [seed] [ipfs_api_url]
const args = process.argv.slice(2);
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';
const HTTP_IPFS_API = args[2] || DEFAULT_IPFS_GATEWAY_URL;
const CHUNK_SIZE = 6 * 1024 // 6 KB

/**
 * Reads metadata JSON from IPFS by metadataCid.
 */
async function retrieveMetadata(metadataCid) {
    console.log(`🧩 Retrieving file from metadataCid: ${metadataCid.toString()}`);

    // 1️⃣ Fetch metadata block
    const metadataBlock = await fetchCid(HTTP_IPFS_API, metadataCid);
    const metadataJson = JSON.parse(new TextDecoder().decode(metadataBlock));
    console.log(`📜 Loaded metadata:`, metadataJson);
    return metadataJson;
}

/**
 * Fetches all chunks listed in metdataJson, concatenates into a single file,
 * and saves to disk (or returns as Buffer).
 */
async function retrieveFileForMetadata(metadataJson, outputPath) {
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
        const block = await fetchCid(HTTP_IPFS_API, chunkCid);
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
export async function storeMetadata(typedApi, signer, chunks) {
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
    console.log(`🧾 Metadata size: ${jsonBytes.length} bytes`);

    // 2️⃣ Store JSON bytes in Bulletin
    const { cid: metadataCid } = await store(typedApi, signer, jsonBytes);
    console.log('🧩 Metadata CID:', metadataCid.toString());

    return { metadataCid };
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
 * @param {CID} expectedRootCid - Expected root CID to verify against
 * @param {CID|string} proofCid - CID of the stored DAG-PB node
 * @param {number} mhCode - Multihash code (default: 0x12 for SHA2-256)
 */
export async function reconstructDagFromProof(expectedRootCid, proofCid, mhCode = 0x12) {
    console.log(`📦 Fetching DAG bytes for proof CID: ${proofCid.toString()}`);

    // 1️⃣ Read the raw block bytes from IPFS
    const dagBytes = await fetchCid(HTTP_IPFS_API, proofCid);

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

async function storeProof(typedApi, whoSigner, rootCID, dagFileBytes) {
    console.log(`🧩 Storing proof for rootCID: ${rootCID.toString()} to the Bulletin`);

    // Store DAG bytes in Bulletin using PAPI store function
    const { cid: rawDagCid } = await store(typedApi, whoSigner, dagFileBytes);
    console.log('📤 DAG proof "bytes" stored in Bulletin with CID:', rawDagCid.toString());

    // This can be a serious pallet, this is just a demonstration.
    const proof = `ProofCid: ${rawDagCid.toString()} -> rootCID: ${rootCID.toString()}`;
    const remarkTx = typedApi.tx.System.remark({ remark: Binary.fromText(proof) });
    await waitForTransaction(remarkTx, whoSigner, "StoreProofRemark");
    console.log(`📤 DAG proof - "${proof}" - stored in Bulletin`);
    return { rawDagCid }
}

async function main() {
    await cryptoWaitReady()

    logHeader('STORE CHUNKED DATA TEST');
    logConnection(NODE_WS, SEED, HTTP_IPFS_API);

    let client, resultCode;
    try {
        const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "bulletin-chunked-"));
        const filePath = path.join(tmpDir, "image.jpeg");
        const out1Path = path.join(tmpDir, "retrieved1.jpeg");
        const out2Path = path.join(tmpDir, "retrieved2.jpeg");
        generateTextImage(filePath, "Hello, Bulletin with PAPI chunked - " + new Date().toString(), "small");

        // Init WS PAPI client and typed api.
        client = createClient(getWsProvider(NODE_WS));
        const bulletinAPI = client.getTypedApi(bulletin);
        const { authorizationSigner, whoSigner, whoAddress } = setupKeyringAndSigners(SEED, '//Chunkedsigner');

        // Authorize an account.
        await authorizeAccount(
            bulletinAPI,
            authorizationSigner,
            whoAddress,
            100,
            BigInt(100 * 1024 * 1024), // 100 MiB
            TX_MODE_FINALIZED_BLOCK
        );

        // Read the file, chunk it, store in Bulletin and return CIDs (using PAPI).
        let { chunks} = await storeChunkedFile(bulletinAPI, whoSigner, filePath, CHUNK_SIZE);

        // Store metadata file with all the CIDs to the Bulletin.
        const { metadataCid} = await storeMetadata(bulletinAPI, whoSigner, chunks);

        ////////////////////////////////////////////////////////////////////////////////////
        // 1. example manually retrieve the picture (no IPFS DAG feature)
        const metadataJson = await retrieveMetadata(metadataCid)
        await retrieveFileForMetadata(metadataJson, out1Path);
        filesAreEqual(filePath, out1Path);

        ////////////////////////////////////////////////////////////////////////////////////
        // 2. example download picture by rootCID with IPFS DAG feature and HTTP gateway.
        // Demonstrates how to download chunked content by one root CID.
        // Basically, just take the `metadataJson` with already stored chunks and convert it to the DAG-PB format.
        const { rootCid, dagBytes } = await buildUnixFSDag(metadataJson, 0xb220)

        // Store DAG and proof to the Bulletin.
        let { rawDagCid } = await storeProof(bulletinAPI, authorizationSigner, rootCid, Buffer.from(dagBytes));
        await reconstructDagFromProof(rootCid, rawDagCid, 0xb220);

        // Store DAG into IPFS.
        assert.strictEqual(
            rootCid.toString(),
            convertCid(rawDagCid, dagPB.code).toString(),
            '❌ DAG CID does not match expected root CID'
        );
        console.log('🧱 DAG stored on IPFS with CID:', rawDagCid.toString())
        console.log('\n🌐 Try opening in browser:')
        console.log(`   ${HTTP_IPFS_API}/ipfs/${rootCid.toString()}`)
        console.log("   (You'll see binary content since this is an image)")
        console.log(`   ${HTTP_IPFS_API}/ipfs/${rawDagCid.toString()}`)
        console.log("   (You'll see the encoded DAG descriptor content)")

        // Download the content from IPFS HTTP gateway
        const fullBuffer = await fetchCid(HTTP_IPFS_API, rootCid);
        console.log(`✅ Reconstructed file size: ${fullBuffer.length} bytes`);
        await fileToDisk(out2Path, fullBuffer);
        filesAreEqual(filePath, out1Path);
        filesAreEqual(out1Path, out2Path);

        // Download the DAG descriptor raw file itself.
        const downloadedDagBytes = await fetchCid(HTTP_IPFS_API, rawDagCid);
        logSuccess(`Downloaded DAG raw descriptor file size: ${downloadedDagBytes.length} bytes`);
        assert.deepStrictEqual(downloadedDagBytes, Buffer.from(dagBytes));
        const dagNode = dagPB.decode(downloadedDagBytes);
        console.log('📄 Decoded DAG node:', dagNode);

        logTestResult(true, 'Store Chunked Data Test');
        resultCode = 0;
    } catch (error) {
        logError(`Error: ${error.message}`);
        console.error(error);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
