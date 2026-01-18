import { createClient } from 'polkadot-api';
import { Enum } from '@polkadot-api/substrate-bindings';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { getPolkadotSigner } from '@polkadot-api/signer';
import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { cidFromBytes, buildUnixFSDagPB, generateTextImage, convertCid, fetchCid } from './common.js';
import {authorizeAccount, store} from './authorize_and_store_papi.js';
import { bulletin } from './.papi/descriptors/dist/index.mjs';
import { withPolkadotSdkCompat } from "polkadot-api/polkadot-sdk-compat"
import assert from "assert";

import fs from 'fs'
import * as dagPB from "@ipld/dag-pb";

// ---- CONFIG ----
const FILE_PATH = './random_picture.jpg'
const OUT_PATH = './retrieved_random_picture.jpg'
const CHUNK_SIZE = 4 * 1024 // 4 KB
// -----------------

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
        const cid = await cidFromBytes(chunk);
        chunks.push({cid, bytes: chunk, len: chunk.length})
    }
    console.log(`✂️ Split into ${chunks.length} chunks`)

    // ---- 2️⃣ Store chunks in Bulletin (expecting just one block) ----
    for (let i = 0; i < chunks.length; i++) {
        const {cid: expectedCid, bytes} = chunks[i]
        console.log(`📤 Storing chunk #${i + 1} CID: ${expectedCid}`)
        let cid = await store(api, pair, bytes, null, null, nonceMgr);
        assert.deepStrictEqual(expectedCid, cid);
        console.log(`✅ Stored chunk #${i + 1} and CID equals!`)
    }
    return { chunks };
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

class NonceManager {
    constructor(initialNonce) {
        this.nonce = initialNonce; // BN instance from api.query.system.account
    }

    getAndIncrement() {
        const current = this.nonce;
        this.nonce = this.nonce + 1; // increment BN
        return current;
    }
}

function filesAreEqual(path1, path2) {
    const data1 = fs.readFileSync(path1);
    const data2 = fs.readFileSync(path2);
    assert.deepStrictEqual(data1.length, data2.length)

    for (let i = 0; i < data1.length; i++) {
        assert.deepStrictEqual(data1[i], data2[i])
    }
}

async function authorizeStorage(api, sudoPair, pair, nonceMgr) {
    // Ensure enough quota.
    const auth = await api.query.TransactionStorage.Authorizations.getValue(Enum("Account", pair.address));
    console.log('Authorization info:', auth)

    if (auth != null) {
        const authValue = auth.extent;
        const transactions = authValue.transactions;
        const bytes = authValue.bytes;

        if (transactions > 10 && bytes > 24 * CHUNK_SIZE) {
            console.log('✅ Account authorization is sufficient.');
            return;
        }
    } else {
        console.log('ℹ️ No existing authorization found — requesting new one...');
    }

    const transactions = 128;
    const bytes = 64 * 1024 * 1024; // 64 MB
    await authorizeAccount(api, sudoPair, pair.address, transactions, bytes, nonceMgr)
}

let client;
async function main() {
    await cryptoWaitReady()
    if (fs.existsSync(FILE_PATH)) {
        fs.unlinkSync(FILE_PATH);
        console.log(`File ${FILE_PATH} removed.`);
    }
    if (fs.existsSync(OUT_PATH)) {
        fs.unlinkSync(OUT_PATH);
        console.log(`File ${OUT_PATH} removed.`);
    }
    generateTextImage(FILE_PATH, "Hello, Bulletin with PAPI - " + new Date().toString());

    console.log('🛰 Connecting to Bulletin node...')
    // Create PAPI client with WebSocket provider
    client = createClient(withPolkadotSdkCompat(getWsProvider('ws://localhost:10000')));
    // Get typed API with generated descriptors
    const typedApi = client.getTypedApi(bulletin);

    // Create keyring and accounts
    const keyring = new Keyring({ type: 'sr25519' });
    const sudoAccount = keyring.addFromUri('//Alice');
    const whoAccount = keyring.addFromUri('//Alice');

    // Create PAPI-compatible signers using @polkadot-api/signer
    const sudoSigner = getPolkadotSigner(
        sudoAccount.publicKey,
        'Sr25519',
        (input) => sudoAccount.sign(input)
    );
    const whoSigner = getPolkadotSigner(
        whoAccount.publicKey,
        'Sr25519',
        (input) => whoAccount.sign(input)
    );

    console.log('✅ Connected to Bulletin node')
    let { nonce } = await typedApi.query.System.Account.getValue(whoAccount.address);
    const nonceMgr = new NonceManager(nonce);
    console.log(`💳 Using account: ${whoAccount.address}, nonce: ${nonce}`)

    // Make sure an account can store data.
    await authorizeStorage(typedApi, sudoSigner, whoAccount, nonceMgr);

    // Read the file, chunk it, store in Bulletin and return CIDs.
    let { chunks} = await storeChunkedFile(typedApi, whoSigner, FILE_PATH, nonceMgr);

    ////////////////////////////////////////////////////////////////////////////////////
    // Example download picture by rootCID with IPFS DAG feature and HTTP gateway.
    // Demonstrates how to download chunked content by one root CID.
    // Basically, just take the `metadataJson` with already stored chunks and convert it to the DAG-PB format.
    const { rootCid: expectedRootCid, dagBytes } = await buildUnixFSDagPB(chunks, 0x12);
    let calculatedRootCid = await cidFromBytes(dagBytes, 0x70, 0x12);
    assert.deepStrictEqual(expectedRootCid, calculatedRootCid);

    // Store DAG file directly to the Bulletin. with DAG-PB / SHA2_256 content_hash.
    // !!! (No IPFS magic needed: ipfs.dag.put or ipfs.block.put(dagBytes, { format: 'dag-pb', mhtype: 'sha2-256'}))
    let rootCid = await store(typedApi, whoSigner, dagBytes, 0x70, 0x12, nonceMgr);
    assert.deepStrictEqual(expectedRootCid, rootCid);

    // Read by rootCID directly over IPFS gateway, which handles download all the chunks.
    // (Other words Bulletin is compatible)
    console.log('🧱 DAG stored on Bulletin with CID:', rootCid.toString())
    console.log('\n🌐 Try opening in browser:')
    console.log(`   http://127.0.0.1:8080/ipfs/${rootCid.toString()}`)
    console.log('   (You’ll see binary content since this is an image)')
    console.log('')
    console.log(`   http://127.0.0.1:8080/ipfs/${convertCid(rootCid, 0x55)}`)
    console.log('   (You’ll see the DAG file itself)')

    // Download the content from IPFS HTTP gateway.
    const fullBuffer = await fetchCid(rootCid);
    console.log(`✅ Reconstructed file size: ${fullBuffer.length} bytes`);
    await fileToDisk(OUT_PATH, fullBuffer);
    filesAreEqual(FILE_PATH, OUT_PATH);

    // Derive CID for DAG content from rootCID (change codec from 0x70 -> 0x55)
    const rootCidAsRaw = convertCid(rootCid, 0x55);
    const storedDagNode = dagPB.decode(await fetchCid(rootCidAsRaw));
    console.log("✅ Reconstructed DAG file: ", storedDagNode);

    console.log(`\n\n\n✅✅✅ Passed all tests ✅✅✅`);
}

main().catch(console.error).finally(() => {
    if (client) client.destroy();
});
