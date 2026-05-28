import fs from 'fs'
import os from 'os'
import path from 'path'
import { cryptoWaitReady } from '@polkadot/util-crypto'
import { CID } from 'multiformats/cid'
import * as dagPB from '@ipld/dag-pb'
import { TextDecoder } from 'util'
import assert from "assert";
import {
    generateTextImage,
    filesAreEqual,
    fileToDisk,
    setupKeyringAndSigners,
    waitForChainReady,
    waitForBlockProduction,
    parseProviderArgs,
    buildProviders,
    DEFAULT_IPFS_GATEWAY_URL,
} from './common.js'
import { logHeader, logConnection, logConfig, logSuccess, logError, logTestResult } from './logger.js'
import { fetchCid } from "./api.js";
import { buildUnixFSDagPB, cidFromBytes, convertCid } from "./cid_dag_metadata.js";
import { Binary } from '@polkadot-api/substrate-bindings';
import { bulletin } from './.papi/descriptors/dist/index.js';
import { BulletinClient } from '../sdk/typescript/dist/index.mjs';

// Command line arguments: [ws_url] [seed] [ipfs_api_url]
const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Eve';
const HTTP_IPFS_API = args[2] || DEFAULT_IPFS_GATEWAY_URL;
const PROVIDER_CFG = parseProviderArgs(process.argv);
const SKIP_IPFS_VERIFY = process.argv.includes('--skip-ipfs-verify');
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
 * Builds metadata describing the file chunks, stores it via the SDK,
 * returns `{ metadataCid }`.
 */
async function storeMetadata(client, chunks) {
    const metadata = {
        type: 'file',
        version: 1,
        totalChunks: chunks.length,
        totalSize: chunks.reduce((a, c) => a + c.len, 0),
        chunks: chunks.map((c, i) => ({
            index: i,
            cid: c.cid.toString(),
            len: c.len,
        })),
    };
    const jsonBytes = Buffer.from(new TextEncoder().encode(JSON.stringify(metadata)));
    console.log(`🧾 Metadata size: ${jsonBytes.length} bytes`);
    const { cids } = await client.upload([{ data: jsonBytes }]).withWaitFor('finalized').send();
    const metadataCid = cids[0];
    console.log('🧩 Metadata CID:', metadataCid.toString());
    return { metadataCid };
}

/**
 * Splits the file into chunks, stores them via the SDK pipeline, and
 * verifies CIDs match the precomputed expectations.
 */
async function storeChunkedFileViaSdk(client, filePath, chunkSize) {
    const fileData = fs.readFileSync(filePath);
    console.log(`📁 Read ${filePath}, size ${fileData.length} bytes`);

    const chunks = [];
    for (let i = 0; i < fileData.length; i += chunkSize) {
        const chunk = fileData.subarray(i, i + chunkSize);
        const cid = await cidFromBytes(chunk);
        chunks.push({ cid, bytes: chunk, len: chunk.length });
    }
    console.log(`✂️ Split into ${chunks.length} chunks`);

    const items = chunks.map((c) => ({ data: c.bytes }));
    const { cids } = await client.upload(items).withWaitFor('finalized').send();
    for (let i = 0; i < chunks.length; i++) {
        assert.deepStrictEqual(
            cids[i].toString(),
            chunks[i].cid.toString(),
            `❌ Chunk #${i + 1} CID mismatch`,
        );
    }
    console.log(`✅ Stored ${chunks.length} chunks; all CIDs verified`);
    return { chunks };
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

// TODO: revisit sudo usage with https://github.com/paritytech/polkadot-bulletin-chain/pull/265
async function storeProof(client, proofSigner, rootCID, dagFileBytes) {
    console.log(`🧩 Storing proof for rootCID: ${rootCID.toString()} to the Bulletin`);

    // Store DAG bytes in Bulletin via the SDK pipeline.
    const { cids } = await client.upload([{ data: dagFileBytes }]).withWaitFor('finalized').send();
    const rawDagCid = cids[0];
    console.log('📤 DAG proof "bytes" stored in Bulletin with CID:', rawDagCid.toString());

    // Demonstration only — System.remark wrapped in Sudo.sudo. Direct PAPI
    // tx via the SDK-exposed typed API; the SDK's pipeline is store-only.
    const proof = `ProofCid: ${rawDagCid.toString()} -> rootCID: ${rootCID.toString()}`;
    const remarkTx = client.api.tx.System.remark({ remark: Binary.fromText(proof) });
    const sudoTx = client.api.tx.Sudo.sudo({ call: remarkTx.decodedCall });
    await new Promise((resolve, reject) => {
        sudoTx.signSubmitAndWatch(proofSigner).subscribe({
            next: (ev) => {
                console.log(`✅ Proof remark event:`, ev.type);
                if (ev.type === 'finalized') resolve();
            },
            error: (err) => {
                console.error(`❌ Proof remark error:`, err);
                reject(err);
            },
        });
    });
    console.log(`📤 DAG proof - "${proof}" - stored in Bulletin`);
    return { rawDagCid };
}

async function main() {
    await cryptoWaitReady()

    logHeader('STORE CHUNKED DATA TEST');
    if (PROVIDER_CFG.mode === 'smoldot') {
        logConfig({
            Mode: 'Smoldot Light Client',
            'Relay Spec': PROVIDER_CFG.relaySpecPath,
            'Para Spec': PROVIDER_CFG.paraSpecPath,
            'IPFS API': HTTP_IPFS_API,
        });
    } else {
        logConnection(NODE_WS, SEED, HTTP_IPFS_API);
    }

    let client, providersHandle, resultCode;
    try {
        const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "bulletin-chunked-"));
        const filePath = path.join(tmpDir, "image.jpeg");
        const out1Path = path.join(tmpDir, "retrieved1.jpeg");
        const out2Path = path.join(tmpDir, "retrieved2.jpeg");
        generateTextImage(filePath, "Hello, Bulletin chunked - " + new Date().toString(), "small");

        providersHandle = await buildProviders({ ...PROVIDER_CFG, wsUrl: NODE_WS });
        const { authorizationSigner, whoSigner, whoAddress } =
            setupKeyringAndSigners(SEED, '//Chunkedsigner');

        client = new BulletinClient({
            descriptor: bulletin,
            providers: providersHandle.providers,
            uploadSigner: whoSigner,
            authorizerSigner: authorizationSigner,
        });

        await waitForChainReady(client.api);
        await waitForBlockProduction(client.api);

        // Pre-fetch the static APIs so PAPI caches metadata/constants
        // before any long-running upload sequence. Without this, a later
        // raw `signSubmitAndWatch` (the sudo.remark demo below) can hit
        // BlockNotPinnedError when PAPI's chainHead-follower has already
        // evicted the finalized block PAPI snapshotted for the validation
        // walk. PAPI's own zombie integration test calls this for the
        // same reason; see
        // https://github.com/polkadot-api/polkadot-api/blob/main/integration-tests/zombie-tests/src/main.spec.ts
        await client.api.getStaticApis();

        // Authorize the chunk-storage account. The proof DAG + System.remark
        // step is dispatched through this same account (no separate proof
        // signer needed since storage is just another upload).
        await client
            .authorizeAccount(whoAddress, 200, BigInt(200 * 1024 * 1024)) // 200 MiB
            .withWaitFor('finalized')
            .send();
        logSuccess(`Authorized ${whoAddress}`);

        // Chunk the file and store all chunks through the SDK pipeline.
        const { chunks } = await storeChunkedFileViaSdk(client, filePath, CHUNK_SIZE);

        // Store metadata describing the chunks.
        const { metadataCid } = await storeMetadata(client, chunks);

        ////////////////////////////////////////////////////////////////////////////////////
        // 1. example manually retrieve the picture (no IPFS DAG feature).
        //    Hits the IPFS HTTP gateway; only runnable when kubo is up.
        if (!SKIP_IPFS_VERIFY) {
            const metadataJson = await retrieveMetadata(metadataCid)
            await retrieveFileForMetadata(metadataJson, out1Path);
            filesAreEqual(filePath, out1Path);
        }

        ////////////////////////////////////////////////////////////////////////////////////
        // 2. UnixFS DAG-PB build from the in-memory chunk list. We don't
        //    need to re-fetch the metadata via IPFS to build the DAG since
        //    we already have the chunks here.
        const { rootCid, dagBytes } = await buildUnixFSDagPB(chunks, 0xb220)

        // Store DAG bytes through `client` (chunk-storage signer is authorized
        // for them) and emit a sudo'd System.remark proof. The sudo call uses
        // `authorizationSigner` which is also the sudo key on dev chains.
        const { rawDagCid } = await storeProof(client, authorizationSigner, rootCid, Buffer.from(dagBytes));
        if (!SKIP_IPFS_VERIFY) {
            await reconstructDagFromProof(rootCid, rawDagCid, 0xb220);
        }

        assert.strictEqual(
            rootCid.toString(),
            convertCid(rawDagCid, dagPB.code).toString(),
            '❌ DAG CID does not match expected root CID'
        );
        console.log('🧱 DAG stored on Bulletin with CID:', rawDagCid.toString())

        if (!SKIP_IPFS_VERIFY) {
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
        }

        logTestResult(true, 'Store Chunked Data Test');
        resultCode = 0;
    } catch (error) {
        logError(`Error: ${error.message}`);
        console.error(error);
        resultCode = 1;
    } finally {
        if (client) await client.destroy();
        if (providersHandle) await providersHandle.cleanup();
        process.exit(resultCode);
    }
}

await main();
