import { ApiPromise, WsProvider } from '@polkadot/api';
import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { create } from 'ipfs-http-client';
import { retrieveMetadata, retrieveFileForMetadata, buildUnixFSDag, waitForNewBlock, filesAreEqual, reconstructDagFromProof, fileToDisk, NonceManager, WS_ENDPOINT, IPFS_API, HTTP_IPFS_API } from "./common";
import { storeChunkedFile, storeMetadata, storeProof, authorizeStorage } from './api';
import fs from 'fs'
import assert from "assert";

// ---- CONFIG ----
const FILE_PATH = './images/32mb-sample.jpg'
const OUT_PATH = './download/retrieved_picture.bin'
const OUT_PATH2 = './download/retrieved_picture.bin2'
// ----

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
    console.log("file path: ", FILE_PATH);
    let { chunks } = await storeChunkedFile(api, pair, FILE_PATH, nonceMgr);
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
