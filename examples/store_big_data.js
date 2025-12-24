import { ApiPromise, WsProvider } from '@polkadot/api';
import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import * as dagPB from '@ipld/dag-pb'
import fs from 'fs'
import assert from "assert";
import { fetchCid } from "./api.js";
import { authorizeStorage, storeChunkedFile, storeMetadata, retrieveMetadata, retrieveFileForMetadata, buildUnixFSDag, waitForNewBlock, generateTextImage, filesAreEqual, storeProof, reconstructDagFromProof, fileToDisk, NonceManager, WS_ENDPOINT, IPFS_API, HTTP_IPFS_API } from "./common";
import { convertCid } from "./cid_dag_metadata";

// ---- CONFIG ----
const FILE_PATH = './images/32mb-sample.jpg'
const OUT_1_PATH = './download/retrieved_picture.bin'
const OUT_2_PATH = './download/retrieved_picture.bin2'
// ----

async function main() {
    await cryptoWaitReady()

    let api, resultCode;
    try {
        if (fs.existsSync(OUT_1_PATH)) {
            fs.unlinkSync(OUT_1_PATH);
            console.log(`File ${OUT_1_PATH} removed.`);
        }
        if (fs.existsSync(OUT_2_PATH)) {
            fs.unlinkSync(OUT_2_PATH);
            console.log(`File ${OUT_2_PATH} removed.`);
        }

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

        // Make sure an account can store data.
        await authorizeStorage(api, sudoPair, pair, nonceMgr);

        // Read the file, chunk it, store in Bulletin and return CIDs.
        let { chunks } = await storeChunkedFile(api, pair, FILE_PATH, nonceMgr);
        // Store metadata file with all the CIDs to the Bulletin.
        const { metadataCid } = await storeMetadata(api, pair, chunks, nonceMgr);
        await waitForNewBlock();

        ////////////////////////////////////////////////////////////////////////////////////
        // 1. example manually retrieve the picture (no IPFS DAG feature)
        const metadataJson = await retrieveMetadata(ipfs, metadataCid);
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
        process.exit(resultCode);
    }
}

await main();
