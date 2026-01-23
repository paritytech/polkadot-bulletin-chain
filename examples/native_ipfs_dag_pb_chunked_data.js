import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { cidFromBytes, buildUnixFSDagPB, convertCid } from './cid_dag_metadata.js';
import { generateTextImage, fileToDisk, filesAreEqual, newSigner, HTTP_IPFS_API } from './common.js';
import { authorizeAccount, store, fetchCid } from './api.js';
import { bulletin } from './.papi/descriptors/dist/index.mjs';
import { withPolkadotSdkCompat } from "polkadot-api/polkadot-sdk-compat"
import assert from "assert";

import fs from 'fs'
import os from 'os'
import path from 'path'
import * as dagPB from "@ipld/dag-pb";

const CHUNK_SIZE = 4 * 1024 // 4 KB

/**
 * Read the file, chunk it, store in Bulletin and return CIDs.
 * Returns { chunks }
 */
async function storeChunkedFile(api, pair, filePath) {
    // ---- 1️⃣ Read and split a file ----
    const fileData = fs.readFileSync(filePath)
    console.log(`📁 Read ${filePath}, size ${fileData.length} bytes`)

    const chunks = []
    for (let i = 0; i < fileData.length; i += CHUNK_SIZE) {
        const chunk = fileData.subarray(i, i + CHUNK_SIZE)
        const cid = await cidFromBytes(chunk);
        chunks.push({ cid, bytes: chunk, len: chunk.length })
    }
    console.log(`✂️ Split into ${chunks.length} chunks`)

    // ---- 2️⃣ Store chunks in Bulletin (expecting just one block) ----
    for (let i = 0; i < chunks.length; i++) {
        const {cid: expectedCid, bytes} = chunks[i]
        console.log(`📤 Storing chunk #${i + 1} CID: ${expectedCid}`)
        let cid = await store(api, pair, bytes);
        assert.deepStrictEqual(expectedCid, cid);
        console.log(`✅ Stored chunk #${i + 1} and CID equals!`)
    }
    return { chunks };
}

async function main() {
    await cryptoWaitReady()

    let client, resultCode;
    try {
        const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "bulletinimggen-"));
        const filePath = path.join(tmpDir, "image.jpeg");
        const downloadedFilePath = path.join(tmpDir, "downloaded.jpeg");
        generateTextImage(filePath, "Hello, Bulletin dag - " + new Date().toString());

        console.log('🛰 Connecting to Bulletin node...')
        // Create PAPI client with WebSocket provider
        client = createClient(withPolkadotSdkCompat(getWsProvider('ws://localhost:10000')));
        // Get typed API with generated descriptors
        const typedApi = client.getTypedApi(bulletin);

        // Create signers
        const { signer: sudoSigner } = newSigner('//Alice');
        const { signer: whoSigner, address: whoAddress } = newSigner('//Nativeipfsdagsigner');

        console.log('✅ Connected to Bulletin node')
        console.log(`💳 Using account: ${whoAddress}`)

        // Make sure an account can store data.
        await authorizeAccount(typedApi, sudoSigner, whoAddress, 128, BigInt(64 * 1024 * 1024));

        // Read the file, chunk it, store in Bulletin and return CIDs.
        let { chunks } = await storeChunkedFile(typedApi, whoSigner, filePath);

        ////////////////////////////////////////////////////////////////////////////////////
        // Example download picture by rootCID with IPFS DAG feature and HTTP gateway.
        // Demonstrates how to download chunked content by one root CID.
        // Basically, just take the `metadataJson` with already stored chunks and convert it to the DAG-PB format.
        const { rootCid: expectedRootCid, dagBytes } = await buildUnixFSDagPB(chunks, 0x12);
        let calculatedRootCid = await cidFromBytes(dagBytes, 0x70, 0x12);
        assert.deepStrictEqual(expectedRootCid, calculatedRootCid);

        // Store DAG file directly to the Bulletin. with DAG-PB / SHA2_256 content_hash.
        // !!! (No IPFS magic needed: ipfs.dag.put or ipfs.block.put(dagBytes, { format: 'dag-pb', mhtype: 'sha2-256'}))
        let rootCid = await store(typedApi, whoSigner, dagBytes, 0x70, 0x12);
        assert.deepStrictEqual(expectedRootCid, rootCid);

        // Read by rootCID directly over IPFS gateway, which handles download all the chunks.
        // (Other words Bulletin is compatible)
        console.log('🧱 DAG stored on Bulletin with CID:', rootCid.toString())
        console.log('\n🌐 Try opening in browser:')
        console.log(`   http://127.0.0.1:8080/ipfs/${rootCid.toString()}`)
        console.log("   (You'll see binary content since this is an image)")
        console.log('')
        console.log(`   http://127.0.0.1:8080/ipfs/${convertCid(rootCid, 0x55)}`)
        console.log("   (You'll see the DAG file itself)")

        // Download the content from IPFS HTTP gateway.
        const fullBuffer = await fetchCid(HTTP_IPFS_API, rootCid);
        console.log(`✅ Reconstructed file size: ${fullBuffer.length} bytes`);
        await fileToDisk(downloadedFilePath, fullBuffer);
        filesAreEqual(filePath, downloadedFilePath);

        // Derive CID for DAG content from rootCID (change codec from 0x70 -> 0x55)
        const rootCidAsRaw = convertCid(rootCid, 0x55);
        const storedDagNode = dagPB.decode(await fetchCid(HTTP_IPFS_API, rootCidAsRaw));
        const decodedDagNode = dagPB.decode(Buffer.from(dagBytes));
        console.log("✅ Reconstructed DAG file: ", storedDagNode);
        assert.deepStrictEqual(storedDagNode, decodedDagNode);

        console.log(`\n\n\n✅✅✅ Passed all tests ✅✅✅`);
        resultCode = 0;
    } catch (error) {
        console.error("❌ Error:", error);
        resultCode = 1;
    } finally {
        if (client) client.destroy();
        process.exit(resultCode);
    }
}

await main();
